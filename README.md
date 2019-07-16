# serde_pipe

[![Crates.io](https://img.shields.io/crates/v/serde_pipe.svg?maxAge=86400)](https://crates.io/crates/serde_pipe)
[![MIT / Apache 2.0 licensed](https://img.shields.io/crates/l/serde_pipe.svg?maxAge=2592000)](#License)
[![Build Status](https://dev.azure.com/alecmocatta/serde_pipe/_apis/build/status/alecmocatta.serde_pipe?branchName=master)](https://dev.azure.com/alecmocatta/serde_pipe/_build/latest?definitionId=1&branchName=master)

[Docs](https://docs.rs/serde_pipe/0.1.0)

Turn serde+bincode into a pipe: push `T`s and pull `u8`s, or vice versa.

This library gives you a `Serializer` pipe, into which you can push `T`s and pull `u8`s; and a `Deserializer` pipe, into which you can push `u8`s and pull `T`s.

Both are bounded in their memory usage, i.e. they do not simply allocate a vector for serde+bincode to serialize into. Instead, [libfringe](https://github.com/edef1c/libfringe) is leveraged to turn serde+bincode into a Generator from which `u8`s can be pulled from/pushed to on demand.

This is useful for example if you have 10GiB memory available, and want to serialize+send or receive+deserialize an 8GiB vector. Note, this is perfectly possible with serde+bincode normally, but you'd need to dedicate a thread that blocks until completion to the task.

## Example

```rust
extern crate serde_pipe;
use serde_pipe::Serializer;

let large_vector = (0..1u64<<30).collect::<Vec<_>>();
let mut serializer = Serializer::new();
serializer.push().unwrap()(large_vector);

while let Some(pull) = serializer.pull() {
	let byte = pull();
	println!("byte! {}", byte);
}
```

## Note

This crate currently depends on [libfringe](https://github.com/edef1c/libfringe), and as such inherits these limitations:
 * Rust nightly is required for the `asm` and `naked_functions` features;
 * The architectures currently supported are: x86, x86_64, aarch64, or1k;
 * The platforms currently supported are: bare metal, Linux (any libc), FreeBSD, DragonFly BSD, macOS. Windows is not supported.

## License
Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE.txt](LICENSE-APACHE.txt) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT.txt](LICENSE-MIT.txt) or http://opensource.org/licenses/MIT)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
