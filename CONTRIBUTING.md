# Contributing to Lithox

First off, thank you for considering contributing to Lithox! It's people like you that make building complex systems in Rust such a rewarding experience.

## Code of Conduct

By participating in this project, you are expected to uphold a welcoming and inclusive environment. Please treat all maintainers and fellow contributors with respect.

## How Can I Contribute?

### 1. Reporting Bugs
If you find a bug in the storage engine (e.g., a deadlock, memory leak, or B+ Tree routing error), please open an issue! Provide as much detail as possible:
* Your OS and Rust version (`rustc --version`).
* A minimal reproducible example, ideally as a small Rust test case.
* The exact panic message or standard error output.

### 2. Suggesting Enhancements
Lithox is being built in distinct phases. If you have an idea for a feature (like WAL logging, an allocation bitmap, or a new SQL parser layer), check the issue tracker first to see if it's already being discussed. If not, open a feature request detailing the proposed architecture.

### 3. Submitting Pull Requests
We welcome PRs for open issues, documentation fixes, and performance improvements!

**Pull Request Workflow:**
1. Fork the repository and create your branch from `main`.
2. Ensure your code follows idiomatic Rust style by running `cargo fmt`.
3. Check for common pitfalls and warnings by running `cargo clippy -- -D warnings`.
4. Ensure all existing systems are unbroken by passing the test suite: `cargo test`.
5. If you are adding a new feature (like a new internal node algorithm), please include a comprehensive test case in the inline `tests` module at the bottom of the relevant file (e.g., `src/bplus_tree.rs`).
6. Write a descriptive commit message and PR description explaining the *why* behind your implementation.

## Architecture Guidelines

If you are modifying the internal database logic, please keep the following constraints in mind:
* **No `Rc` or `Box` pointers:** The B+ Tree relies strictly on `PageId` routing to navigate the Buffer Pool.
* **Strict 4KB Pages:** All node structs must cleanly cast to and from `[u8; 4096]` byte arrays using `bytemuck`.
* **Zero-Copy Where Possible:** Avoid cloning page data; use `RwLock` read/write latches to mutate pages directly in the buffer pool.

Thank you for helping Lithox become a rock-solid, production-grade Rust storage engine!