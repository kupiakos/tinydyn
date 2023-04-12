# `tinydyn`

Tiny dynamic dispatch for embedded use.

`Ref<dyn Trait>` and `RefMut<dyn Trait>` wrap a pointer and metadata necessary to call
trait methods, and `Deref` into a _tinydyn trait object_ that implements the `Trait`.

Traits must currently opt-in by annotating with `#[tinydyn]`.
This defines an alternate, lighter weight vtable, and if the trait has one method, eliminates
it entirely by putting the function pointer inline.
This does not affect normal behavior of the trait, and can still be made into a `dyn Trait`.
This, however, would be wasteful.

# Example

```rust
use tinydyn::{tinydyn, Ref};
#[tinydyn]
trait Foo {
    fn blah(&self) -> i32;
    fn blue(&self) -> i32 { 10 }
}
impl Foo for i32 {
    fn blah(&self) -> i32 { *self + 1 }
}

// Like upcasting to `&dyn Foo`, but with a tinydyn vtable.
let x: Ref<dyn Foo> = Ref::new(&15);
assert_eq!(x.blah(), 16);
assert_eq!(x.blue(), 10);
```

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for details.

## License

Apache 2.0; see [`LICENSE`](LICENSE) for details.

## Disclaimer

This project is not an official Google project. It is not supported by
Google and Google specifically disclaims all warranties as to its quality,
merchantability, or fitness for a particular purpose.