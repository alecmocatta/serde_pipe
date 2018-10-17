//! Turn serde+bincode into a pipe: push `T`s and pull `u8`s, or vice versa.
//!
//! **[Crates.io](https://crates.io/crates/serde_pipe) │ [Repo](https://github.com/alecmocatta/serde_pipe)**
//!
//! This library gives you a `Serializer` pipe, into which you can push `T`s and pull `u8`s; and a `Deserializer` pipe, into which you can push `u8`s and pull `T`s.
//!
//! Both are bounded in their memory usage, i.e. they do not simply allocate a vector for serde+bincode to serialize into. Instead, [libfringe](https://github.com/edef1c/libfringe) is leveraged to turn serde+bincode into a Generator from which `u8`s can be pulled from/pushed to on demand.
//!
//! This is useful for example if you have 10GiB memory available, and want to serialize+send or receive+deserialize an 8GiB vector. Note, this is perfectly possible with serde+bincode normally, but you'd need to dedicate a thread that blocks until completion to the task.
//!
//! # Example
//!
//! ```rust,no_run
//! extern crate serde_pipe;
//! use serde_pipe::Serializer;
//!
//! let large_vector = (0..1u64<<30).collect::<Vec<_>>();
//! let mut serializer = Serializer::new();
//! serializer.push().unwrap()(large_vector);
//!
//! while let Some(pull) = serializer.pull() {
//! 	let byte = pull();
//! 	println!("byte! {}", byte);
//! }
//! ```
//!
//! # Note
//!
//! This crate currently depends on [libfringe](https://github.com/edef1c/libfringe), and as such inherits these limitations:
//!  * Rust nightly is required for the `asm` and `naked_functions` features;
//!  * The architectures currently supported are: x86, x86_64, aarch64, or1k;
//!  * The platforms currently supported are: bare metal, Linux (any libc), FreeBSD, DragonFly BSD, macOS. Windows is not supported.

#![doc(html_root_url = "https://docs.rs/serde_pipe/0.1.0")]
#![feature(pin, nll)]
#![warn(
	missing_copy_implementations,
	missing_debug_implementations,
	missing_docs,
	trivial_numeric_casts,
	unused_extern_crates,
	unused_import_braces,
	unused_qualifications,
	unused_results,
	clippy::pedantic
)] // from https://github.com/rust-unofficial/patterns/blob/master/anti_patterns/deny-warnings.md
#![allow(
	clippy::items_after_statements,
	clippy::inline_always,
	clippy::new_without_default_derive,
	clippy::boxed_local
)]

extern crate bincode;
extern crate either;
extern crate fringe;
#[cfg(test)]
extern crate rand;
extern crate serde;

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
							Counter(t, 0)
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
	fn as_any_ref(&self) -> &Any;
	fn as_any_mut(&mut self) -> &mut Any;
	fn as_any_box(self: Box<Self>) -> Box<Any>;
}
impl<T: serde::ser::Serialize + 'static> SerializerInnerBox for SerializerInner<T> {
	fn next_box(&mut self) -> Option<u8> {
		self.next()
	}
	fn into_stack_box(self: Box<Self>) -> fringe::OsStack {
		self.into_stack()
	}
	fn as_any_ref(&self) -> &Any {
		self as &Any
	}
	fn as_any_mut(&mut self) -> &mut Any {
		self as &mut Any
	}
	fn as_any_box(self: Box<Self>) -> Box<Any> {
		self as Box<Any>
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
/// ```rust,no_run
/// extern crate serde_pipe;
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
	serializer: Option<Box<SerializerInnerBox>>,
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
				if self.serializer.is_none() || !self
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
				if ret.is_none() {
					self.done = true;
				}
				self.pull = ret;
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
#[doc(hidden)]
impl marker::Unpin for Serializer {}
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
						Counter(t, 0)
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
	fn as_any_ref(&self) -> &Any;
	fn as_any_mut(&mut self) -> &mut Any;
	fn as_any_box(self: Box<Self>) -> Box<Any>;
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
	fn as_any_ref(&self) -> &Any {
		self as &Any
	}
	fn as_any_mut(&mut self) -> &mut Any {
		self as &mut Any
	}
	fn as_any_box(self: Box<Self>) -> Box<Any> {
		self as Box<Any>
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
/// ```rust
/// extern crate serde_pipe;
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
	deserializer: Option<Box<DeserializerInnerBox>>,
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
			if self.deserializer.is_none() || !self
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
			assert!(
				!self
					.deserializer
					.as_mut()
					.unwrap()
					.as_any_mut()
					.downcast_mut::<DeserializerInner<T>>()
					.unwrap()
					.done()
			);
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
#[doc(hidden)]
impl marker::Unpin for Deserializer {}
impl fmt::Debug for Deserializer {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.debug_struct("Deserializer")
			.field("done", &self.done)
			.field("pending", &self.pending)
			.field("mid", &self.mid)
			.finish()
	}
}

#[cfg(test)]
mod tests {
	#![allow(
		clippy::cyclomatic_complexity,
		clippy::let_unit_value,
		clippy::collapsible_if
	)]

	use super::*;
	use rand::{prng::XorShiftRng, Rng, SeedableRng};
	use std::{collections::VecDeque, io};

	struct VecDequeWriter<'a>(&'a mut VecDeque<u8>);
	impl<'a> Write for VecDequeWriter<'a> {
		#[inline(always)]
		fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
			for &byte in buf {
				self.0.push_back(byte);
			}
			Ok(buf.len())
		}
		#[inline(always)]
		fn flush(&mut self) -> io::Result<()> {
			Ok(())
		}
	}
	enum Queue {
		Unit,
		U8(u8),
		U16(u16),
		U32(u32),
		U64(u64),
		String(String),
	}

	#[test]
	fn serializer() {
		let mut rng =
			XorShiftRng::from_seed([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
		let iterations = if cfg!(debug_assertions) { 5_000 } else { 50_000 }; // hack until https://internals.rust-lang.org/t/idea-allow-to-query-current-optimization-level-using-cfg-opt-level/7089
		for _ in 0..iterations {
			let mut serializer = Serializer::new();
			let mut queue = VecDeque::new();
			for _ in 0..rng.gen_range(0, 10_000) {
				if rng.gen() {
					match rng.gen_range(0, 6) {
						0 => {
							if let Some(push) = serializer.push() {
								let mut y = vec![];
								bincode::serialize_into(&mut y, &()).unwrap();
								assert_eq!(y, vec![]);
								queue.push_back(0);
								push(());
							}
						}
						1 => {
							if let Some(push) = serializer.push() {
								let x: u8 = rng.gen();
								bincode::serialize_into(&mut VecDequeWriter(&mut queue), &x)
									.unwrap();
								push(x);
							}
						}
						2 => {
							if let Some(push) = serializer.push() {
								let x: u16 = rng.gen();
								bincode::serialize_into(&mut VecDequeWriter(&mut queue), &x)
									.unwrap();
								push(x);
							}
						}
						3 => {
							if let Some(push) = serializer.push() {
								let x: u32 = rng.gen();
								bincode::serialize_into(&mut VecDequeWriter(&mut queue), &x)
									.unwrap();
								push(x);
							}
						}
						4 => {
							if let Some(push) = serializer.push() {
								let x: u64 = rng.gen();
								bincode::serialize_into(&mut VecDequeWriter(&mut queue), &x)
									.unwrap();
								push(x);
							}
						}
						5 => {
							if let Some(push) = serializer.push() {
								let x: String = rng.gen::<usize>().to_string();
								bincode::serialize_into(&mut VecDequeWriter(&mut queue), &x)
									.unwrap();
								push(x);
							}
						}
						_ => unreachable!(),
					}
				} else {
					if let Some(pull) = serializer.pull() {
						let x: u8 = pull();
						assert_eq!(queue.pop_front().unwrap(), x);
					}
				}
			}
			// if let Some(ref mut empty) = serializer.empty() { https://github.com/rust-lang/rust/issues/52706
			// 	empty();
			// }
			let empty = serializer.empty();
			if empty.is_some() {
				assert_ne!(queue.len(), 0);
				empty.unwrap()();
			} else {
				assert_eq!(queue, vec![]);
			}
		}
	}

	#[test]
	fn deserializer() {
		let mut rng =
			XorShiftRng::from_seed([15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0]);
		let iterations = if cfg!(debug_assertions) { 5_000 } else { 50_000 }; // hack until https://internals.rust-lang.org/t/idea-allow-to-query-current-optimization-level-using-cfg-opt-level/7089
		for _ in 0..iterations {
			let mut deserializer = Deserializer::new();
			let mut queue = VecDeque::new();
			let mut pipe = VecDeque::new();
			for _ in 0..rng.gen_range(0, 10_000) {
				match rng.gen_range(0, 3) {
					0 => {
						if pipe.len() < 100 {
							match rng.gen_range(0, 6) {
								0 => {
									let mut y = vec![];
									bincode::serialize_into(&mut y, &()).unwrap();
									assert_eq!(y, vec![]);
									pipe.push_back(0);
									queue.push_back(Queue::Unit);
								}
								1 => {
									let x: u8 = rng.gen();
									bincode::serialize_into(&mut VecDequeWriter(&mut pipe), &x)
										.unwrap();
									queue.push_back(Queue::U8(x));
								}
								2 => {
									let x: u16 = rng.gen();
									bincode::serialize_into(&mut VecDequeWriter(&mut pipe), &x)
										.unwrap();
									queue.push_back(Queue::U16(x));
								}
								3 => {
									let x: u32 = rng.gen();
									bincode::serialize_into(&mut VecDequeWriter(&mut pipe), &x)
										.unwrap();
									queue.push_back(Queue::U32(x));
								}
								4 => {
									let x: u64 = rng.gen();
									bincode::serialize_into(&mut VecDequeWriter(&mut pipe), &x)
										.unwrap();
									queue.push_back(Queue::U64(x));
								}
								5 => {
									let x: String = rng.gen::<usize>().to_string();
									bincode::serialize_into::<_, String>(
										&mut VecDequeWriter(&mut pipe),
										&x,
									)
									.unwrap();
									queue.push_back(Queue::String(x.clone()));
								}
								_ => unreachable!(),
							}
						}
					}
					1 => {
						if let (Some(_), Some(push)) = (pipe.front(), deserializer.push()) {
							push(pipe.pop_front().unwrap());
						}
					}
					2 => {
						if let Some(front) = queue.front() {
							match *front {
								Queue::Unit => {
									if let Some(pull) = deserializer.pull() {
										let () = pull();
										let _ = queue.pop_front().unwrap();
									}
								}
								Queue::U8(q) => {
									if let Some(pull) = deserializer.pull() {
										let x: u8 = pull();
										assert_eq!(x, q);
										let _ = queue.pop_front().unwrap();
									}
								}
								Queue::U16(q) => {
									if let Some(pull) = deserializer.pull() {
										let x: u16 = pull();
										assert_eq!(x, q);
										let _ = queue.pop_front().unwrap();
									}
								}
								Queue::U32(q) => {
									if let Some(pull) = deserializer.pull() {
										let x: u32 = pull();
										assert_eq!(x, q);
										let _ = queue.pop_front().unwrap();
									}
								}
								Queue::U64(q) => {
									if let Some(pull) = deserializer.pull() {
										let x: u64 = pull();
										assert_eq!(x, q);
										let _ = queue.pop_front().unwrap();
									}
								}
								Queue::String(ref q) => {
									if let Some(pull) = deserializer.pull() {
										let x: String = pull();
										assert_eq!(&x, q);
										let _ = queue.pop_front().unwrap();
									}
								}
							}
						}
					}
					_ => unreachable!(),
				}
			}
			let empty = deserializer.empty();
			if empty.is_some() {
				empty.unwrap()();
			}
		}
	}

	#[test]
	fn both() {
		let mut rng = XorShiftRng::from_seed([0, 1, 2, 3, 4, 5, 6, 7, 7, 6, 5, 4, 3, 2, 1, 0]);
		let iterations = if cfg!(debug_assertions) { 5_000 } else { 50_000 }; // hack until https://internals.rust-lang.org/t/idea-allow-to-query-current-optimization-level-using-cfg-opt-level/7089
		for _ in 0..iterations {
			let mut serializer = Serializer::new();
			let mut deserializer = Deserializer::new();
			let mut queue = VecDeque::new();
			for _ in 0..rng.gen_range(0, 10_000) {
				match rng.gen_range(0, 3) {
					0 => match rng.gen_range(0, 6) {
						0 => {
							if let Some(push) = serializer.push() {
								let mut y = vec![];
								bincode::serialize_into(&mut y, &()).unwrap();
								assert_eq!(y, vec![]);
								queue.push_back(Queue::Unit);
								push(());
							}
						}
						1 => {
							if let Some(push) = serializer.push() {
								let x: u8 = rng.gen();
								queue.push_back(Queue::U8(x));
								push(x);
							}
						}
						2 => {
							if let Some(push) = serializer.push() {
								let x: u16 = rng.gen();
								queue.push_back(Queue::U16(x));
								push(x);
							}
						}
						3 => {
							if let Some(push) = serializer.push() {
								let x: u32 = rng.gen();
								queue.push_back(Queue::U32(x));
								push(x);
							}
						}
						4 => {
							if let Some(push) = serializer.push() {
								let x: u64 = rng.gen();
								queue.push_back(Queue::U64(x));
								push(x);
							}
						}
						5 => {
							if let Some(push) = serializer.push() {
								let x: String = rng.gen::<usize>().to_string();
								queue.push_back(Queue::String(x.clone()));
								push(x);
							}
						}
						_ => unreachable!(),
					},
					1 => {
						if let (Some(pull), Some(push)) = (serializer.pull(), deserializer.push()) {
							push(pull());
						}
					}
					2 => {
						if let Some(front) = queue.front() {
							match *front {
								Queue::Unit => {
									if let Some(pull) = deserializer.pull() {
										let () = pull();
										let _ = queue.pop_front().unwrap();
									}
								}
								Queue::U8(q) => {
									if let Some(pull) = deserializer.pull() {
										let x: u8 = pull();
										assert_eq!(x, q);
										let _ = queue.pop_front().unwrap();
									}
								}
								Queue::U16(q) => {
									if let Some(pull) = deserializer.pull() {
										let x: u16 = pull();
										assert_eq!(x, q);
										let _ = queue.pop_front().unwrap();
									}
								}
								Queue::U32(q) => {
									if let Some(pull) = deserializer.pull() {
										let x: u32 = pull();
										assert_eq!(x, q);
										let _ = queue.pop_front().unwrap();
									}
								}
								Queue::U64(q) => {
									if let Some(pull) = deserializer.pull() {
										let x: u64 = pull();
										assert_eq!(x, q);
										let _ = queue.pop_front().unwrap();
									}
								}
								Queue::String(ref q) => {
									if let Some(pull) = deserializer.pull() {
										let x: String = pull();
										assert_eq!(&x, q);
										let _ = queue.pop_front().unwrap();
									}
								}
							}
						}
					}
					_ => unreachable!(),
				}
			}
			let empty = serializer.empty();
			if empty.is_some() {
				empty.unwrap()();
			}
			let empty = deserializer.empty();
			if empty.is_some() {
				empty.unwrap()();
			}
		}
	}
}
