# Contributing

Four questions before a change goes in. They are the checks CI enforces, written
out so the answer is obvious before the push.

1. **Does every new field have a reader outside tests?** A config field the
   runtime ignores is worse than a missing feature, because it lies to whoever
   sets it. Wire it or leave it out.
2. **Does any comment cite a past?** *Legacy*, *deprecated*, *backwards
   compatible*, *retained* and *formerly* are claims that an older revision
   exists. Cite the commit, or delete the shim.
3. **Does the README still match?** The compiler and integration tests catch
   code drift; only a human catches a sentence that is no longer true.
4. **Is this the only way we do this?** One codec, one `host:port` parser, one
   defaulting rule, one span function, one error policy, one test-naming style.
   A second way to do a thing is a bug waiting to diverge.

## Running the checks

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo machete
```

## Questions & Contact

For inquiries, security disclosures, or direct contact with the engineering team at macrodataware, email [engineers@macrodataware.com](mailto:engineers@macrodataware.com) or visit [https://macrodataware.com/](https://macrodataware.com/).
