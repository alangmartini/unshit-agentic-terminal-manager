# Clipboard tests

Normal test runs must not overwrite the developer's system clipboard. Tests
that read, write, or clear the real clipboard are opt-in with Rust's `#[ignore]`
attribute. Pure content and event tests remain part of the default suite.

Run the system clipboard tests explicitly on a disposable desktop session:

```sh
cargo test -p unshit-app --features clipboard --lib clipboard::tests -- --ignored --test-threads=1
cargo test -p unshit-test --features unshit-app/clipboard --test clipboard -- --ignored --test-threads=1
```

From the terminal manager workspace, the app's clipboard dispatch tests are
also opt-in:

```sh
cargo test -p terminal-manager --bin terminal-manager terminal_paste -- --ignored --test-threads=1
cargo test -p terminal-manager --bin terminal-manager terminal_copy -- --ignored --test-threads=1
cargo test -p terminal-manager --bin terminal-manager terminal_export_info -- --ignored --test-threads=1
cargo test -p terminal-manager --bin terminal-manager quick_prompt_image_paste -- --ignored --test-threads=1
cargo test -p terminal-manager --bin terminal-manager explorer_copy -- --ignored --test-threads=1
```

These commands replace clipboard contents, including text, images, and file
lists. They do not restore previous contents. Run the commands sequentially;
the test mutexes do not serialize access across separate processes.
