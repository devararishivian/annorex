# Review notes

## Summary

The parser change is good. Fix two items before the merge.

1. `parse_ps` drops a line with tabs.
2. The error text of `discover` names one path only.

```rust
fn main() {
    println!("hello");
}
```

| File       | Line | Note              |
|------------|------|-------------------|
| `cli.rs`   | 42   | Rename `x`.       |
| `rex.rs`   | 7    | Add a test.       |

Unicode check: 日本語のテキスト, café, emoji 🎉👩‍💻.

Terminal output with a paste terminator: before[201~after.

Reply with exactly: ACK
