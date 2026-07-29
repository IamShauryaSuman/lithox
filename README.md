# Lithox 🦀🪨

[![Rust CI](https://github.com/IamShauryaSuman/lithox/actions/workflows/rust.yml/badge.svg)](https://github.com/IamShauryaSuman/lithox/actions)
[![Crates.io](https://img.shields.io/crates/v/lithox)](https://crates.io/crates/lithox)
[![License](https://img.shields.io/crates/l/lithox)](https://crates.io/crates/lithox)

**Lithox** is a persistent, on-disk B+ Tree storage engine and buffer pool manager written entirely in Rust. It serves as the foundational storage and indexing layer for a relational database, designed to handle datasets larger than available RAM while ensuring memory safety and concurrency.

## 🚀 Current Status

**Phase 1 (Storage), Phase 2 (Memory Management), Phase 3 (Serialization), and Phase 4 (B+ Tree Logic) are complete.** 
The database can currently allocate, read, and write 4KB pages to disk, efficiently cache them using a Clock Replacer, and perform high-performance, zero-copy serialization of complex B+ Tree nodes over raw memory blocks using `bytemuck`. It features a fully functioning B+ Tree that supports recursive splitting, self-healing deletion (borrowing, internal merging, and root collapsing), and infinite O(1) page recycling using an on-disk intrusive linked list.

**Next up:** 
* **v0.2.0:** Database REPL (Interactive command-line interface)
* **v0.3.0:** Lock Crabbing (Advanced multithreaded concurrency)

## 🏗️ Architecture overview

Lithox is built bottom-up in distinct layers:
1. **Disk Manager:** Handles raw file I/O operations, mapping logical `PageId`s to 4KB blocks on disk.
2. **Clock Replacer:** An O(1) approximation of LRU that tracks which memory frames are safe to evict.
3. **Buffer Pool Manager:** The orchestrator that moves pages between disk and memory, tracking dirty pages and pin counts.
4. **B+ Tree Index:** The primary data structure, leveraging `PageId` pointers instead of memory pointers to bypass Rust's strict borrow-checker graph limitations.

## 🛠️ Usage

To run the internal test suite and verify the engine's functionality:

```bash
cargo test
```

To run the linter and ensure idiomatic Rust:

```bash
cargo clippy
```

## 📖 Specification
For a detailed breakdown of the page layouts, API structures, and development phases, please refer to [SPEC.md](SPEC.md).

## 📄 License
This project is licensed under the MIT or Apache-2.0 License.