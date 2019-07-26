use std::{
	any::TypeId, fmt, io::{self, Read}, mem
};

struct ReadCounter<T: Read>(T, usize);
impl<T: Read> ReadCounter<T> {
	fn new(t: T) -> Self {
		Self(t, 0)
	}
	fn count(&self) -> usize {
		self.1
	}
}
impl<T: Read> Read for ReadCounter<T> {
	#[inline(always)]
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		self.0.read(buf).map(|x| {
			self.1 += x;
			x
		})
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
	buffer: Option<(Box<[u8]>, usize)>,
}
impl Serializer {
	/// Construct a new Serializer pipe.
	#[inline(always)]
	pub fn new() -> Self {
		Self { buffer: None }
	}

	#[doc(hidden)]
	pub fn push_avail(&self) -> bool {
		self.buffer.is_none()
	}
	/// Push a `T` to the Serializer pipe. [`None`] denotes that the Serializer is instead awaiting a [`pull`](Serializer::pull()). [`Some`] contains an `impl FnOnce(T)` that can be called to perform the `push`.
	pub fn push<'a, T: serde::ser::Serialize + 'static>(
		&'a mut self,
	) -> Option<impl FnOnce(T) + 'a> {
		if self.buffer.is_none() {
			Some(move |t| {
				let mut vec = vec![0; mem::size_of::<usize>()];
				bincode::serialize_into(&mut vec, &t).unwrap();
				let mut len = vec.len() - mem::size_of::<usize>();
				if len == 0 {
					len += 1;
					vec.push(0);
				}
				let mut len_vec = Vec::with_capacity(mem::size_of::<usize>());
				bincode::serialize_into::<_, usize>(&mut len_vec, &len).unwrap();
				vec[..mem::size_of::<usize>()].copy_from_slice(&len_vec);
				self.buffer = Some((vec.into_boxed_slice(), 0));
			})
		} else {
			None
		}
	}

	#[doc(hidden)]
	pub fn pull_avail(&self) -> bool {
		self.buffer.is_some()
	}
	/// Pull a `T` from the Serializer pipe. [`None`] denotes that the Serializer is instead awaiting a [`push`](Serializer::push()). [`Some`] contains an `impl FnOnce() -> u8` that can be called to perform the `pull`.
	pub fn pull<'a>(&'a mut self) -> Option<impl FnOnce() -> u8 + 'a> {
		if self.buffer.is_some() {
			Some(move || {
				let (buffer, index) = self.buffer.as_mut().unwrap();
				let ret = buffer[*index];
				*index += 1;
				if *index == buffer.len() {
					self.buffer = None;
				}
				ret
			})
		} else {
			None
		}
	}

	#[doc(hidden)]
	pub fn empty_avail(&self) -> bool {
		self.buffer.is_some()
	}
	/// Empty this pipe. [`None`] denotes it's already empty. [`Some`] contains an `impl FnOnce()` that can be called to perform the empty.
	pub fn empty<'a>(&'a mut self) -> Option<impl FnOnce() + 'a> {
		if self.buffer.is_some() {
			Some(move || {
				self.buffer = None;
			})
		} else {
			None
		}
	}
}
impl Drop for Serializer {
	#[inline(always)]
	fn drop(&mut self) {
		assert!(self.buffer.is_none());
	}
}
impl fmt::Debug for Serializer {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.debug_struct("Serializer")
			.field("buffer", &self.buffer)
			.finish()
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
	buffer: Vec<u8>,
	len: usize,
	deserializer: Option<TypeId>,
}
impl Deserializer {
	/// Construct a new Deserializer pipe.
	#[inline(always)]
	pub fn new() -> Self {
		Self {
			buffer: Vec::with_capacity(9),
			len: 0,
			deserializer: None,
		}
	}

	#[doc(hidden)]
	pub fn pull_avail(&self) -> bool {
		self.len != 0 && self.buffer.len() == self.len
	}
	/// Pull a `T` from the Deserializer pipe. [`None`] denotes that the Deserializer is instead awaiting a [`push`](Deserializer::push()). [`Some`] contains an `impl FnOnce() -> T` that can be called to perform the `pull`.
	///
	/// Note that [`push`](Deserializer::push()) will return [`None`] until [`pull`](Deserializer::pull()) has been called, as it's necessary to supply the type of the value being seserialized.
	pub fn pull<'a, T: serde::de::DeserializeOwned + 'static>(
		&'a mut self,
	) -> Option<impl FnOnce() -> T + 'a> {
		let deserializer = TypeId::of::<T>();
		if self.deserializer.is_none() {
			self.deserializer = Some(deserializer);
		}
		assert_eq!(self.deserializer.unwrap(), deserializer);
		if self.len != 0 && self.buffer.len() == self.len {
			Some(move || {
				let mut counter = ReadCounter::new(&*self.buffer);
				let ret = bincode::deserialize_from(&mut counter).unwrap();
				let mut len = counter.count();
				if len == 0 {
					len += 1;
					assert_eq!(self.buffer[0], 0);
				}
				assert_eq!(len, self.len);
				self.len = 0;
				self.deserializer = None;
				self.buffer.clear();
				ret
			})
		} else {
			None
		}
	}

	#[doc(hidden)]
	pub fn push_avail(&self) -> bool {
		self.deserializer.is_some() && (self.buffer.len() != self.len || self.len == 0)
	}
	/// Push a `u8` to the Deserializer pipe. [`None`] denotes that the Deserializer is instead awaiting a [`pull`](Deserializer::pull()). [`Some`] contains an `impl FnOnce(u8)` that can be called to perform the `push`.
	///
	/// Note that [`push`](Deserializer::push()) will return [`None`] until [`pull`](Deserializer::pull()) has been called, as it's necessary to supply the type of the value being seserialized.
	pub fn push<'a>(&'a mut self) -> Option<impl FnOnce(u8) + 'a> {
		if self.deserializer.is_some() && (self.buffer.len() != self.len || self.len == 0) {
			Some(move |x| {
				self.buffer.push(x);
				if self.len == 0 && self.buffer.len() == mem::size_of::<usize>() {
					let mut counter = ReadCounter::new(&*self.buffer);
					self.len = bincode::deserialize_from::<_, usize>(&mut counter).unwrap();
					assert_eq!(counter.count(), mem::size_of::<usize>());
					self.buffer.clear();
					self.buffer.reserve(self.len);
				}
			})
		} else {
			None
		}
	}

	#[doc(hidden)]
	pub fn empty_avail(&self) -> bool {
		!self.buffer.is_empty() || self.len != 0 || self.deserializer.is_some()
	}
	/// Empty this pipe. [`None`] denotes it's already empty. [`Some`] contains an `impl FnOnce()` that can be called to perform the empty.
	pub fn empty<'a>(&'a mut self) -> Option<impl FnOnce() + 'a> {
		if !self.buffer.is_empty() || self.len != 0 || self.deserializer.is_some() {
			Some(move || {
				self.buffer.clear();
				self.len = 0;
				self.deserializer = None;
			})
		} else {
			None
		}
	}
}
impl Drop for Deserializer {
	#[inline(always)]
	fn drop(&mut self) {
		assert!(self.buffer.is_empty() && self.len == 0 && self.deserializer.is_none());
	}
}
impl fmt::Debug for Deserializer {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		f.debug_struct("Deserializer")
			.field("buffer", &self.buffer)
			.field("len", &self.len)
			.field("deserializer", &self.deserializer)
			.finish()
	}
}
