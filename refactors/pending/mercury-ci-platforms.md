# building mercury on isograph's platforms

isograph's CI builds its compiler for five Rust targets:

- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`
- `x86_64-pc-windows-msvc`

mercury cannot match three of them. It grabs the keyboard through a `CGEventTap`, draws a status item through `AppKit`, and binds both through `objc2`, none of which exist off macOS. So the Linux and Windows targets are not mercury's to build; for freddie they are the portable crates', and `freddie-cli-off-macos.md` already puts `freddie_cli` and `freddie_single_instance` in CI on `ubuntu-latest` and `windows-latest`.

What mercury shares with isograph is the two macOS targets. freddie's CI builds one of them and not the other.

## What CI builds today

`cargo test --all --all-features` runs on `macos-latest`, which is an arm64 runner, so mercury is built and tested for `aarch64-apple-darwin` and nothing else. `x86_64-apple-darwin`, which isograph builds and ships, is never compiled, so a change that breaks the Intel build lands green.

## The change

A job that compiles the workspace for `x86_64-apple-darwin` on the same `macos-latest` runner, cross from arm64. It builds and does not run: an x64 test binary would need Rosetta, and the point is compile parity, which is also all isograph asks of a non-native target.

```yaml
  cargo-build-intel:
    name: cargo build (x86_64-apple-darwin)
    runs-on: macos-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install Rust
        uses: actions-rust-lang/setup-rust-toolchain@v1
        with:
          toolchain: "1.96.0"
          target: x86_64-apple-darwin
      # Build, not test: the runner is arm64, so this cross-compiles to catch an Intel-only break
      # in mercury or a crate under it. Tests run on the native target in `cargo-test`.
      - name: Build for Intel
        run: cargo build --workspace --all-targets --all-features --target x86_64-apple-darwin
```

`all-checks-passed` gains it:

```yaml
    needs: [cargo-fmt, cargo-clippy, cargo-test, cargo-build-intel, extension, build-website]
```

## The other three targets are not mercury's

`freddie-cli-off-macos.md` adds a `portable` job that builds and tests `freddie_cli` and `freddie_single_instance` on `ubuntu-latest` and `windows-latest`. That is the Linux and Windows half of isograph's matrix, on the OSes rather than the exact targets: `ubuntu-latest` is glibc x64, not the two musl targets isograph cross-builds, and neither doc adds `aarch64-unknown-linux-musl`.

The musl and arm64-Linux targets exist in isograph's CI because it ships a static binary per platform to users who install the compiler. freddie ships no per-platform binary: `freddie_cli` is a library, and mercury runs only where it was built. So the portable crates need to prove they compile and test on each OS, which `ubuntu-latest` and `windows-latest` do, and matching isograph's musl and cross machinery would buy nothing until freddie distributes a binary for those platforms. When it does, this is where that target list grows.

## The changes, in order

1. **`cargo-build-intel`.** The job above, and its name added to `all-checks-passed`'s `needs`. This is the whole of what mercury can add toward isograph's matrix; the Linux and Windows coverage is `freddie-cli-off-macos.md`'s.
