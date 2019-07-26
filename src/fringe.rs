use either::Either;
use std::{
	any::Any, fmt, io::{self, Read, Write}, marker
};

#[derive(Debug)]
enum SerializerMsg<T> {
	Kill,
	Next,
	New(T),
}
struct SerializerInner<T: serde::ser::Serialize + 'static> {
	generator: Option<
		fringe::generator::Generator<'static, SerializerMsg<T>, Option<u8>, fringe::OsStack>,
	>,
	_marker: marker::PhantomData<fn(T)>,
}
/// These are I believe safe, as there's almost certainly nothing !Send on the stack, at least nothing that crosses the boundary; and all access is mediated through &mut self
unsafe impl<T: serde::ser::Serialize + 'static> Send for SerializerInner<T> {}
unsafe impl<T: serde::ser::Serialize + 'static> Sync for SerializerInner<T> {}
impl<T: serde::ser::Serialize + 'static> SerializerInner<T> {
	#[inline(always)]
	fn new(stack: Option<fringe::OsStack>) -> Self {
		let stack = stack.unwrap_or_else(|| fringe::OsStack::new(64 * 1024).unwrap());
		let generator = fringe::generator::Generator::<SerializerMsg<T>, Option<u8>, _>::new(
			stack,
			move |yielder, t| {
				let mut x = Some(t);
				while let Some(t) = match x.take().unwrap_or_else(|| yielder.suspend(None)) {
					SerializerMsg::New(t) => Some(t),
					SerializerMsg::Kill => None,
					_ => panic!(),
				} {
					if let SerializerMsg::Next = yielder.suspend(None) {
					} else {
						panic!()
					}
					struct Writer<'a, T: 'a>(
						&'a fringe::generator::Yielder<SerializerMsg<T>, Option<u8>>,
					);
					impl<'a, T: 'a> Write for Writer<'a, T> {
						#[inline(always)]
						fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
							for byte in buf {
								if let SerializerMsg::Next = self.0.suspend(Some(*byte)) {
								} else {
									panic!()
								};
							}
							Ok(buf.len())
						}
						#[inline(always)]
						fn flush(&mut self) -> io::Result<()> {
							Ok(())
						}
					}
					struct Counter<T: Write>(T, usize);
					impl<T: Write> Counter<T> {
						fn new(t: T) -> Self {
							Self(t, 0)
						}
						fn count(&self) -> usize {
							self.1
						}
					}
					impl<T: Write> Write for Counter<T> {
						#[inline(always)]
						fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
							self.1 += buf.len();
							self.0.write(buf)
						}
						#[inline(always)]
						fn flush(&mut self) -> io::Result<()> {
							self.0.flush()
						}
					}
					let mut writer = Writer(yielder);
					let mut counter = Counter::new(&mut writer);
					bincode::serialize_into(&mut counter, &t).unwrap();
					if counter.count() == 0 {
						let _ = writer.write(&[0]).unwrap();
					}
				}
			},
		);
		Self {
			generator: Some(generator),
			_marker: marker::PhantomData,
		}
	}

	#[inline(always)]
	fn push(&mut self, t: T) {
		let x = self
			.generator
			.as_mut()
			.unwrap()
			.resume(SerializerMsg::New(t))
			.unwrap();
		assert!(x.is_none());
	}

	#[inline(always)]
	fn next(&mut self) -> Option<u8> {
		self.generator
			.as_mut()
			.unwrap()
			.resume(SerializerMsg::Next)
			.unwrap()
	}

	#[inline(always)]
	fn into_stack(mut self) -> fringe::OsStack {
		let mut generator = self.generator.take().unwrap();
		let x = generator.resume(SerializerMsg::Kill);
		assert!(x.is_none());
		generator.unwrap()
	}
}
impl<T: serde::ser::Serialize + 'static> Drop for SerializerInner<T> {
	#[inline(always)]
	fn drop(&mut self) {
		if let Some(mut generator) = self.generator.take() {
			let x = generator.resume(SerializerMsg::Kill);
			assert!(x.is_none());
			let _ = generator.unwrap();
		}
	}
}
trait SerializerInnerBox: Send + Sync {
	fn next_box(&mut self) -> Option<u8>;
	fn into_stack_box(self: Box<Self>) -> fringe::OsStack;
	fn as_any_ref(&self) -> &dyn Any;
	fn as_any_mut(&mut self) -> &mut dyn Any;
	fn as_any_box(self: Box<Self>) -> Box<dyn Any>;
}
impl<T: serde::ser::Serialize + 'static> SerializerInnerBox for SerializerInner<T> {
	fn next_box(&mut self) -> Option<u8> {
		self.next()
	}
	fn into_stack_box(self: Box<Self>) -> fringe::OsStack {
		self.into_stack()
	}
	fn as_any_ref(&self) -> &dyn Any {
		self as &dyn Any
	}
	fn as_any_mut(&mut self) -> &mut dyn Any {
		self as &mut dyn Any
	}
	fn as_any_box(self: Box<Self>) -> Box<dyn Any> {
		self as Box<dyn Any>
	}
}

/// Serializer pipe: push `T`; pull `u8`.
///
/// The [`push`](Serializer::push()) and [`pull`](Serializer::pull()) calls can signify "blocking" – i.e. they're awaiting the other call – by returning [`None`].
///
/// A [`Some`] returned signifies readiness, holding an `impl FnOnce` that you can call to perform the push/pull.
///
/// # Example
///
/// ```no_run
/// use serde_pipe::Serializer;
///
/// let large_vector = (0..1u64<<30).collect::<Vec<_>>();
/// let mut serializer = Serializer::new();
/// serializer.push().unwrap()(large_vector);
///
/// while let Some(pull) = serializer.pull() {
/// 	let byte = pull();
/// 	println!("byte! {}", byte);
/// }
/// ```
///
/// # Panics
///
/// Will panic if dropped while non-empty. In practise this almost always signifies a bug. If you do want to drop it when non-empty, call [`Serializer::empty()`] before dropping it.
pub struct Serializer {
	serializer: Option<Box<dyn SerializerInnerBox>>,
	done: bool,
	pull: Option<u8>,
}
impl Serializer {
	/// Construct a new Serializer pipe.
	#[inline(always)]
	pub fn new() -> Self {
		Self {
			serializer: None,
			done: true,
			pull: None,
		}
	}

	#[doc(hidden)]
	pub fn push_avail(&self) -> bool {
		self.done
	}
	/// Push a `T` to the Serializer pipe. [`None`] denotes that the Serializer is instead awaiting a [`pull`](Serializer::pull()). [`Some`] contains an `impl FnOnce(T)` that can be called to perform the `push`.
	pub fn push<'a, T: serde::ser::Serialize + 'static>(
		&'a mut self,
	) -> Option<impl FnOnce(T) + 'a> {
		if self.done {
			Some(move |t| {
				self.done = false;
				if self.serializer.is_none()
					|| !self
						.serializer
						.as_ref()
						.unwrap()
						.as_any_ref()
						.is::<SerializerInner<T>>()
				{
					self.serializer = Some(Box::new(SerializerInner::<T>::new(
						self.serializer.take().map(|x| x.into_stack_box()),
					)));
				}
				self.serializer
					.as_mut()
					.unwrap()
					.as_any_mut()
					.downcast_mut::<SerializerInner<T>>()
					.unwrap()
					.push(t);
				let ret = self.serializer.as_mut().unwrap().next_box();
				self.pull = Some(ret.unwrap());
			})
		} else {
			None
		}
	}

	#[doc(hidden)]
	pub fn pull_avail(&self) -> bool {
		self.pull.is_some()
	}
	/// Pull a `T` from the Serializer pipe. [`None`] denotes that the Serializer is instead awaiting a [`push`](Serializer::push()). [`Some`] contains an `impl FnOnce() -> u8` that can be called to perform the `pull`.
	pub fn pull<'a>(&'a mut self) -> Option<impl FnOnce() -> u8 + 'a> {
		if self.pull.is_some() {
			Some(move || {
				let ret = self.pull.take().unwrap();
				if !self.done {
					let ret = self.serializer.as_mut().unwrap().next_box();
					if ret.is_none() {
						self.done = true;
					}
					self.pull = ret;
				}
				ret
			})
		} else {
			None
		}
	}

	#[doc(hidden)]
	pub fn empty_avail(&self) -> bool {
		!self.done || self.pull.is_some()
	}
	/// Empty this pipe. [`None`] denotes it's already empty. [`Some`] contains an `impl FnOnce()` that can be called to perform the empty.
	pub fn empty<'a>(&'a mut self) -> Option<impl FnOnce() + 'a> {
		if !self.done || self.pull.is_some() {
			Some(move || {
				if !self.done {
					while self.serializer.as_mut().unwrap().next_box().is_some() {}
					self.done = true;
				}
				self.pull = None;
			})
		} else {
			None
		}
	}
}
impl Drop for Serializer {
	#[inline(always)]
	fn drop(&mut self) {
		assert!(self.done && self.pull.is_none());
	}
}
impl Unpin for Serializer {}
impl fmt::Debug for Serializer {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.debug_struct("Serializer")
			.field("done", &self.done)
			.field("pull", &self.pull.is_some())
			.finish()
	}
}

#[derive(Debug)]
enum DeserializerMsg {
	Empty,
	Kill,
	Next,
	New(u8),
}
struct DeserializerInner<T: serde::de::DeserializeOwned + 'static> {
	generator: Option<
		fringe::generator::Generator<'static, DeserializerMsg, Either<bool, T>, fringe::OsStack>,
	>,
	_marker: marker::PhantomData<fn() -> T>,
}
/// These are I believe safe, as there's almost certainly nothing !Send on the stack, at least nothing that crosses the boundary; and all access is mediated through &mut self
unsafe impl<T: serde::de::DeserializeOwned + 'static> Send for DeserializerInner<T> {}
unsafe impl<T: serde::de::DeserializeOwned + 'static> Sync for DeserializerInner<T> {}
impl<T: serde::de::DeserializeOwned + 'static> DeserializerInner<T> {
	#[inline(always)]
	fn new(stack: Option<fringe::OsStack>) -> Self {
		let stack = stack.unwrap_or_else(|| fringe::OsStack::new(64 * 1024).unwrap());
		let generator = fringe::generator::Generator::new(stack, move |yielder, t| {
			let mut x = Some(t);
			loop {
				let t = match x
					.take()
					.unwrap_or_else(|| yielder.suspend(Either::Left(false)))
				{
					DeserializerMsg::New(t) => Some(t),
					DeserializerMsg::Next => None,
					DeserializerMsg::Kill => break,
					DeserializerMsg::Empty => panic!(),
				};
				enum Abc<T> {
					Item(T),
					Kill,
					Empty,
				}
				struct Reader<'a, T: 'a>(
					&'a fringe::generator::Yielder<DeserializerMsg, Either<bool, T>>,
					Option<u8>,
					usize,
					Option<bool>,
				);
				impl<'a, T: 'a> Read for Reader<'a, T> {
					#[inline(always)]
					fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
						if let Some(killed) = self.3 {
							return Err(io::Error::new(
								if killed {
									io::ErrorKind::BrokenPipe
								} else {
									io::ErrorKind::UnexpectedEof
								},
								"",
							));
						}
						for byte in buf.iter_mut() {
							let mut x;
							while {
								x = self.1.take().map(Abc::Item).or_else(|| {
									match self.0.suspend(Either::Left(false)) {
										DeserializerMsg::New(t) => Some(Abc::Item(t)),
										DeserializerMsg::Next => None,
										DeserializerMsg::Kill if self.2 == 0 => Some(Abc::Kill),
										DeserializerMsg::Kill => panic!("{}", self.2),
										DeserializerMsg::Empty if self.2 > 0 => Some(Abc::Empty),
										DeserializerMsg::Empty => panic!(),
									}
								});
								x.is_none()
							} {}
							let x = x.unwrap();
							match x {
								Abc::Item(x) => *byte = x,
								Abc::Kill => {
									self.3 = Some(true);
									return Err(io::Error::new(io::ErrorKind::BrokenPipe, ""));
								}
								Abc::Empty => {
									self.3 = Some(false);
									return Err(io::Error::new(io::ErrorKind::UnexpectedEof, ""));
								}
							}
							self.2 += 1;
						}
						Ok(buf.len())
					}
				}
				struct Counter<T: Read>(T, usize);
				impl<T: Read> Counter<T> {
					fn new(t: T) -> Self {
						Self(t, 0)
					}
					fn count(&self) -> usize {
						self.1
					}
				}
				impl<T: Read> Read for Counter<T> {
					#[inline(always)]
					fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
						self.0.read(buf).map(|x| {
							self.1 += x;
							x
						})
					}
				}
				let mut reader = Reader(yielder, t, 0, None);
				let mut counter = Counter::new(&mut reader);
				let ret: Result<T, _> = bincode::deserialize_from(&mut counter);
				if let Err(err) = ret {
					match *err {
						bincode::ErrorKind::Io(ref err)
							if err.kind() == io::ErrorKind::BrokenPipe =>
						{
							break
						}
						bincode::ErrorKind::Io(ref err)
							if err.kind() == io::ErrorKind::UnexpectedEof =>
						{
							x = Some(yielder.suspend(Either::Left(false)));
							continue;
						}
						_ => unreachable!(),
					}
				}
				let ret = ret.unwrap();
				if counter.count() == 0 {
					let mut x;
					while {
						x = reader.1.take().map(Some).or_else(|| {
							match yielder.suspend(Either::Left(false)) {
								DeserializerMsg::New(t) => Some(Some(t)),
								DeserializerMsg::Next => None,
								DeserializerMsg::Kill => Some(None),
								DeserializerMsg::Empty => unreachable!(),
							}
						});
						x.is_none()
					} {}
					let x = x.unwrap();
					if let Some(x) = x {
						assert_eq!(x, 0);
					} else {
						break;
					}
				}
				if let DeserializerMsg::Next = yielder.suspend(Either::Left(false)) {
				} else {
					panic!()
				};
				if let DeserializerMsg::Next = yielder.suspend(Either::Left(true)) {
				} else {
					panic!()
				};
				x = Some(yielder.suspend(Either::Right(ret)));
			}
		});
		Self {
			generator: Some(generator),
			_marker: marker::PhantomData,
		}
	}

	#[inline(always)]
	fn done(&mut self) -> bool {
		self.generator
			.as_mut()
			.unwrap()
			.resume(DeserializerMsg::Next)
			.unwrap()
			.left()
			.unwrap()
	}

	#[inline(always)]
	fn empty(&mut self) {
		let x = self
			.generator
			.as_mut()
			.unwrap()
			.resume(DeserializerMsg::Empty)
			.unwrap();
		assert!(!x.left().unwrap());
	}

	#[inline(always)]
	fn retrieve(&mut self) -> T {
		self.generator
			.as_mut()
			.unwrap()
			.resume(DeserializerMsg::Next)
			.unwrap()
			.right()
			.unwrap()
	}
	#[inline(always)]
	fn discard(&mut self) {
		let _ = self
			.generator
			.as_mut()
			.unwrap()
			.resume(DeserializerMsg::Next)
			.unwrap()
			.right()
			.unwrap();
	}

	#[inline(always)]
	fn next(&mut self, x: u8) {
		let x = self
			.generator
			.as_mut()
			.unwrap()
			.resume(DeserializerMsg::New(x))
			.unwrap();
		assert!(!x.left().unwrap());
	}

	#[inline(always)]
	fn into_stack(mut self) -> fringe::OsStack {
		let mut generator = self.generator.take().unwrap();
		let x = generator.resume(DeserializerMsg::Kill);
		assert!(x.is_none());
		generator.unwrap()
	}
}
impl<T: serde::de::DeserializeOwned + 'static> Drop for DeserializerInner<T> {
	#[inline(always)]
	fn drop(&mut self) {
		if let Some(mut generator) = self.generator.take() {
			let x = generator.resume(DeserializerMsg::Kill);
			assert!(x.is_none());
			let _ = generator.unwrap();
		}
	}
}
trait DeserializerInnerBox: Send + Sync {
	fn next_box(&mut self, x: u8);
	fn done_box(&mut self) -> bool;
	fn empty_box(&mut self);
	fn discard_box(&mut self);
	fn into_stack_box(self: Box<Self>) -> fringe::OsStack;
	fn as_any_ref(&self) -> &dyn Any;
	fn as_any_mut(&mut self) -> &mut dyn Any;
	fn as_any_box(self: Box<Self>) -> Box<dyn Any>;
}
impl<T: serde::de::DeserializeOwned + 'static> DeserializerInnerBox for DeserializerInner<T> {
	fn next_box(&mut self, x: u8) {
		self.next(x)
	}
	fn done_box(&mut self) -> bool {
		self.done()
	}
	fn empty_box(&mut self) {
		self.empty()
	}
	fn discard_box(&mut self) {
		self.discard()
	}
	fn into_stack_box(self: Box<Self>) -> fringe::OsStack {
		self.into_stack()
	}
	fn as_any_ref(&self) -> &dyn Any {
		self as &dyn Any
	}
	fn as_any_mut(&mut self) -> &mut dyn Any {
		self as &mut dyn Any
	}
	fn as_any_box(self: Box<Self>) -> Box<dyn Any> {
		self as Box<dyn Any>
	}
}

/// Deserializer pipe: push `u8`; pull `T`.
///
/// You will not be able to push any `u8` until [`Deserializer::pull::<T>()`](Deserializer::pull()) has been called to specify the type to be deserialized to.
///
/// The [`push`](Deserializer::push()) and [`pull`](Deserializer::pull()) calls can signify "blocking" – i.e. they're awaiting the other call – by returning [`None`].
///
/// A [`Some`] returned signifies readiness, holding an `impl FnOnce` that you can call to perform the push/pull.
///
/// # Example
///
/// ```
/// use serde_pipe::{Serializer,Deserializer};
///
/// let large_vector = (0..1u64<<10).collect::<Vec<_>>();
/// let mut serializer = Serializer::new();
/// serializer.push().unwrap()(large_vector);
///
/// let mut deserializer = Deserializer::new();
/// deserializer.pull::<Vec<u64>>();
///
/// while let Some(pull) = serializer.pull() {
/// 	let byte = pull();
/// 	deserializer.push().unwrap()(byte);
/// }
///
/// let large_vector = deserializer.pull::<Vec<u64>>().unwrap()();
/// ```
///
/// # Panics
///
/// Will panic if dropped while non-empty. In practise this almost always signifies a bug. If you do want to drop it when non-empty, call [`Deserializer::empty()`] before dropping it.
pub struct Deserializer {
	deserializer: Option<Box<dyn DeserializerInnerBox>>,
	done: bool,
	pending: bool,
	mid: bool,
}
impl Deserializer {
	/// Construct a new Deserializer pipe.
	#[inline(always)]
	pub fn new() -> Self {
		Self {
			deserializer: None,
			done: true,
			pending: false,
			mid: false,
		}
	}

	#[doc(hidden)]
	pub fn pull_avail(&self) -> bool {
		self.pending
	}
	/// Pull a `T` from the Deserializer pipe. [`None`] denotes that the Deserializer is instead awaiting a [`push`](Deserializer::push()). [`Some`] contains an `impl FnOnce() -> T` that can be called to perform the `pull`.
	///
	/// Note that [`push`](Deserializer::push()) will return [`None`] until [`pull`](Deserializer::pull()) has been called, as it's necessary to supply the type of the value being seserialized.
	pub fn pull<'a, T: serde::de::DeserializeOwned + 'static>(
		&'a mut self,
	) -> Option<impl FnOnce() -> T + 'a> {
		if self.done {
			self.done = false;
			if self.deserializer.is_none()
				|| !self
					.deserializer
					.as_ref()
					.unwrap()
					.as_any_ref()
					.is::<DeserializerInner<T>>()
			{
				self.deserializer = Some(Box::new(DeserializerInner::<T>::new(
					self.deserializer.take().map(|x| x.into_stack_box()),
				)));
			}
			assert!(!self
				.deserializer
				.as_mut()
				.unwrap()
				.as_any_mut()
				.downcast_mut::<DeserializerInner<T>>()
				.unwrap()
				.done());
		}
		if self.pending {
			Some(move || {
				self.pending = false;
				self.done = true;
				self.deserializer
					.as_mut()
					.unwrap()
					.as_any_mut()
					.downcast_mut::<DeserializerInner<T>>()
					.unwrap()
					.retrieve()
			})
		} else {
			None
		}
	}

	#[doc(hidden)]
	pub fn push_avail(&self) -> bool {
		!self.done && !self.pending
	}
	/// Push a `u8` to the Deserializer pipe. [`None`] denotes that the Deserializer is instead awaiting a [`pull`](Deserializer::pull()). [`Some`] contains an `impl FnOnce(u8)` that can be called to perform the `push`.
	///
	/// Note that [`push`](Deserializer::push()) will return [`None`] until [`pull`](Deserializer::pull()) has been called, as it's necessary to supply the type of the value being seserialized.
	pub fn push<'a>(&'a mut self) -> Option<impl FnOnce(u8) + 'a> {
		if !self.done && !self.pending {
			Some(move |x| {
				self.mid = true;
				self.deserializer.as_mut().unwrap().next_box(x);
				if self.deserializer.as_mut().unwrap().done_box() {
					self.mid = false;
					self.pending = true;
				}
			})
		} else {
			None
		}
	}

	#[doc(hidden)]
	pub fn empty_avail(&self) -> bool {
		self.mid || self.pending
	}
	/// Empty this pipe. [`None`] denotes it's already empty. [`Some`] contains an `impl FnOnce()` that can be called to perform the empty.
	pub fn empty<'a>(&'a mut self) -> Option<impl FnOnce() + 'a> {
		if self.mid || self.pending {
			Some(move || {
				if self.pending {
					self.deserializer.as_mut().unwrap().discard_box();
					self.pending = false;
				}
				if self.mid {
					self.deserializer.as_mut().unwrap().empty_box();
					self.mid = false;
				}
				self.done = true;
			})
		} else {
			None
		}
	}
}
impl Drop for Deserializer {
	#[inline(always)]
	fn drop(&mut self) {
		assert!(!self.mid && !self.pending);
	}
}
impl Unpin for Deserializer {}
impl fmt::Debug for Deserializer {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.debug_struct("Deserializer")
			.field("done", &self.done)
			.field("pending", &self.pending)
			.field("mid", &self.mid)
			.finish()
	}
}
