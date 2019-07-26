# serde_pipe

[![Crates.io](https://img.shields.io/crates/v/serde_pipe.svg?maxAge=86400)](https://crates.io/crates/serde_pipe)
[![MIT / Apache 2.0 licensed](https://img.shields.io/crates/l/serde_pipe.svg?maxAge=2592000)](#License)
[![Build Status](https://dev.azure.com/alecmocatta/serde_pipe/_apis/build/status/tests?branchName=master)](https://dev.azure.com/alecmocatta/serde_pipe/_build/latest?definitionId=1&branchName=master)

[Docs](https://docs.rs/serde_pipe/0.1.1)

Turn serde+bincode into a pipe: push `T`s and pull `u8`s, or vice versa.

This library gives you a `Serializer` pipe, into which you can push `T`s and pull `u8`s; and a `Deserializer` pipe, into which you can push `u8`s and pull `T`s.

This by default works by allocating a vector to hold the intermediate `u8`s. However the `fringe` feature can be enabled, which uses [libfringe](https://github.com/edef1c/libfringe) to turn serde+bincode into a Generator, resulting in bounded memory usage.

## Example

```rust
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

The `fringe` feature depends on [libfringe](https://github.com/edef1c/libfringe), and so enabling it inherits these limitations:
 * Rust nightly is required for the `asm` and `naked_functions` features;
 * The architectures currently supported are: x86, x86_64, aarch64, or1k;
 * The platforms currently supported are: bare metal, Linux (any libc), FreeBSD, DragonFly BSD, macOS. Windows is not supported.

## License
Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE.txt](LICENSE-APACHE.txt) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT.txt](LICENSE-MIT.txt) or http://opensource.org/licenses/MIT)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
