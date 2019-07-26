//! Turn serde+bincode into a pipe: push `T`s and pull `u8`s, or vice versa.
//!
//! **[Crates.io](https://crates.io/crates/serde_pipe) │ [Repo](https://github.com/alecmocatta/serde_pipe)**
//!
//! This library gives you a `Serializer` pipe, into which you can push `T`s and pull `u8`s; and a `Deserializer` pipe, into which you can push `u8`s and pull `T`s.
//!
//! This by default works by allocating a vector to hold the intermediate `u8`s. However the `fringe` feature can be enabled, which uses [libfringe](https://github.com/edef1c/libfringe) to turn serde+bincode into a Generator, resulting in bounded memory usage.
//!
//! # Example
//!
//! ```no_run
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
//! The `fringe` feature depends on [libfringe](https://github.com/edef1c/libfringe), and so enabling it inherits these limitations:
//!  * Rust nightly is required for the `asm` and `naked_functions` features;
//!  * The architectures currently supported are: x86, x86_64, aarch64, or1k;
//!  * The platforms currently supported are: bare metal, Linux (any libc), FreeBSD, DragonFly BSD, macOS. Windows is not supported.

#![doc(html_root_url = "https://docs.rs/serde_pipe/0.1.1")]
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
	clippy::new_without_default,
	clippy::boxed_local
)]

#[cfg(not(feature = "fringe"))]
mod buffer;
#[cfg(not(feature = "fringe"))]
pub use crate::buffer::*;
#[cfg(feature = "fringe")]
mod fringe;
#[cfg(feature = "fringe")]
pub use crate::fringe::*;

#[cfg(test)]
mod tests {
	#![allow(
		clippy::cognitive_complexity,
		clippy::let_unit_value,
		clippy::collapsible_if
	)]

	use super::*;
	use rand::{rngs::SmallRng, Rng, SeedableRng};
	use std::{
		collections::VecDeque, io::{self, Write}
	};

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
		let mut rng = SmallRng::from_seed([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
		// hack until https://internals.rust-lang.org/t/idea-allow-to-query-current-optimization-level-using-cfg-opt-level/7089
		let iterations = if cfg!(debug_assertions) {
			5_000
		} else {
			50_000
		};
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
								#[cfg(not(feature = "fringe"))]
								bincode::serialize_into::<_, usize>(
									&mut VecDequeWriter(&mut queue),
									&1,
								)
								.unwrap();
								queue.push_back(0);
								push(());
							}
						}
						1 => {
							if let Some(push) = serializer.push() {
								let x: u8 = rng.gen();
								#[cfg(not(feature = "fringe"))]
								bincode::serialize_into::<_, usize>(
									&mut VecDequeWriter(&mut queue),
									&1,
								)
								.unwrap();
								bincode::serialize_into(&mut VecDequeWriter(&mut queue), &x)
									.unwrap();
								push(x);
							}
						}
						2 => {
							if let Some(push) = serializer.push() {
								let x: u16 = rng.gen();
								#[cfg(not(feature = "fringe"))]
								bincode::serialize_into::<_, usize>(
									&mut VecDequeWriter(&mut queue),
									&2,
								)
								.unwrap();
								bincode::serialize_into(&mut VecDequeWriter(&mut queue), &x)
									.unwrap();
								push(x);
							}
						}
						3 => {
							if let Some(push) = serializer.push() {
								let x: u32 = rng.gen();
								#[cfg(not(feature = "fringe"))]
								bincode::serialize_into::<_, usize>(
									&mut VecDequeWriter(&mut queue),
									&4,
								)
								.unwrap();
								bincode::serialize_into(&mut VecDequeWriter(&mut queue), &x)
									.unwrap();
								push(x);
							}
						}
						4 => {
							if let Some(push) = serializer.push() {
								let x: u64 = rng.gen();
								#[cfg(not(feature = "fringe"))]
								bincode::serialize_into::<_, usize>(
									&mut VecDequeWriter(&mut queue),
									&8,
								)
								.unwrap();
								bincode::serialize_into(&mut VecDequeWriter(&mut queue), &x)
									.unwrap();
								push(x);
							}
						}
						5 => {
							if let Some(push) = serializer.push() {
								let x: String = rng.gen::<usize>().to_string();
								#[cfg(not(feature = "fringe"))]
								bincode::serialize_into::<_, usize>(
									&mut VecDequeWriter(&mut queue),
									&(8 + x.len()),
								)
								.unwrap();
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
			if let Some(empty) = serializer.empty() {
				assert_ne!(queue.len(), 0);
				empty();
			} else {
				assert_eq!(queue, vec![]);
			};
		}
	}

	#[test]
	fn deserializer() {
		let mut rng = SmallRng::from_seed([15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0]);
		// hack until https://internals.rust-lang.org/t/idea-allow-to-query-current-optimization-level-using-cfg-opt-level/7089
		let iterations = if cfg!(debug_assertions) {
			5_000
		} else {
			50_000
		};
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
									#[cfg(not(feature = "fringe"))]
									bincode::serialize_into::<_, usize>(
										&mut VecDequeWriter(&mut pipe),
										&1,
									)
									.unwrap();
									pipe.push_back(0);
									queue.push_back(Queue::Unit);
								}
								1 => {
									let x: u8 = rng.gen();
									#[cfg(not(feature = "fringe"))]
									bincode::serialize_into::<_, usize>(
										&mut VecDequeWriter(&mut pipe),
										&1,
									)
									.unwrap();
									bincode::serialize_into(&mut VecDequeWriter(&mut pipe), &x)
										.unwrap();
									queue.push_back(Queue::U8(x));
								}
								2 => {
									let x: u16 = rng.gen();
									#[cfg(not(feature = "fringe"))]
									bincode::serialize_into::<_, usize>(
										&mut VecDequeWriter(&mut pipe),
										&2,
									)
									.unwrap();
									bincode::serialize_into(&mut VecDequeWriter(&mut pipe), &x)
										.unwrap();
									queue.push_back(Queue::U16(x));
								}
								3 => {
									let x: u32 = rng.gen();
									#[cfg(not(feature = "fringe"))]
									bincode::serialize_into::<_, usize>(
										&mut VecDequeWriter(&mut pipe),
										&4,
									)
									.unwrap();
									bincode::serialize_into(&mut VecDequeWriter(&mut pipe), &x)
										.unwrap();
									queue.push_back(Queue::U32(x));
								}
								4 => {
									let x: u64 = rng.gen();
									#[cfg(not(feature = "fringe"))]
									bincode::serialize_into::<_, usize>(
										&mut VecDequeWriter(&mut pipe),
										&8,
									)
									.unwrap();
									bincode::serialize_into(&mut VecDequeWriter(&mut pipe), &x)
										.unwrap();
									queue.push_back(Queue::U64(x));
								}
								5 => {
									let x: String = rng.gen::<usize>().to_string();
									#[cfg(not(feature = "fringe"))]
									bincode::serialize_into::<_, usize>(
										&mut VecDequeWriter(&mut pipe),
										&(8 + x.len()),
									)
									.unwrap();
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
			if let Some(empty) = deserializer.empty() {
				empty();
			};
		}
	}

	#[test]
	fn both() {
		let mut rng = SmallRng::from_seed([0, 1, 2, 3, 4, 5, 6, 7, 7, 6, 5, 4, 3, 2, 1, 0]);
		// hack until https://internals.rust-lang.org/t/idea-allow-to-query-current-optimization-level-using-cfg-opt-level/7089
		let iterations = if cfg!(debug_assertions) {
			5_000
		} else {
			50_000
		};
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
			if let Some(empty) = serializer.empty() {
				empty();
			};
			if let Some(empty) = deserializer.empty() {
				empty();
			};
		}
	}
}
