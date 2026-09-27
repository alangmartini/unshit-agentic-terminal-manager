# Clipboard tests

Normal test runs must not overwrite the developer's system clipboard. Tests
that read, write, or clear the real clipboard are opt-in with Rust's `#[ignore]`
attribute. Pure content and event tests remain part of the default suite.

Run the system clipboard tests explicitly on a disposable desktop session:

```sh
cargo test -p unshit-app --features clipboard --lib clipboard::tests -- --ignored --test-threads=1
cargo test -p unshit-test --features unshit-app/clipboard --test clipboard -- --ignored --test-threads=1
```

These commands replace clipboard contents, including text, images, and file
lists. They do not restore previous contents. Run the commands sequentially;
the test mutexes do not serialize access across separate processes.
