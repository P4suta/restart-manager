# Restart-manager

Find and restart Windows processes locking files from Rust.

## Installation

```sh
cargo add restart-manager
```

## Usage

```rust
use restart_manager::RestartSession;

fn main() -> Result<(), restart_manager::Error> {
    let mut session = RestartSession::new()?;
    session.register_files(["Cargo.toml"])?;
    let applications = session.affected_applications()?;
    println!("Affected applications: {}", applications.len());
    session.end()
}
```

## Documentation

[Read the documentation](docs/README.md).

## License

Licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the [Massachusetts Institute of Technology (MIT) license](LICENSE-MIT), at your option.
You license each contribution you submit under the same two licenses, without other terms, unless you state otherwise.
