// Copyright 2023 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Lightweight dynamic dispatch, intended for embedded use.
//!
//! [`Ref<dyn Trait>`] and [`RefMut<dyn Trait>`] wrap a pointer and metadata necessary to call
//! trait methods, and [`Deref`] into a _tinydyn trait object_ that implements the `Trait`.
//!
//! Traits must currently opt-in by annotating with [`tinydyn`].
//! This defines an alternate, lighter weight [vtable], and if the trait has one method, eliminates
//! it entirely by putting the function pointer inline.
//! This does not affect normal behavior of the trait, and can still be made into a `dyn Trait`.
//! This, however, would be wasteful.
//!
//! [vtable]: https://en.wikipedia.org/wiki/Virtual_method_table
//!
//! # Example
//!
//! ```ignore  TODO figure out why this is broken
//! use tinydyn::{tinydyn, Ref, RefMut};
//!
//! #[tinydyn]
//! trait Spam {
//!     fn ham(&mut self) -> i32;
//!     fn eggs(&self) -> i32 { 10 }
//! }
//!
//! impl Spam for i32 {
//!     fn ham(&self) -> i32 {
//!         *self += 2;
//!         *self - 1
//!     }
//! }
//!
//! let mut x = 15;
//!
//! // Like upcasting to `&dyn Foo`, but with a lighter weight vtable.
//! let mut mutable: RefMut<dyn Foo> = RefMut::new(&mut x);
//! assert_eq!(mutable.ham(), 16);
//! assert_eq!(mutable.eggs(), 10);
//!
//! // mutable.into() would instead consume and have the same lifetime as `mutable`
//! let shared: Ref<dyn Foo> = mutable.as_ref();
//! assert_eq!(shared.eggs(), 10);
//! // Cannot call `shared.ham()` as it's a shared ref and can't call `&mut self` methods.
//!
//! assert_eq!(x, 17);
//! ```
//!
//! ## Planned features
//!
//! ⚠️ **This library is not yet tested enough to be production ready** ⚠️
//!
//! - [x] `&self` and `&mut self` methods
//! - [x] `+ Send` and `+ Sync` trait objects
//! - [x] lifetime `where` bounds on methods
//! - [x] lifetime generics on methods
//! - [ ] implementing on foreign traits/custom vtables
//! - [ ] implementations for common `core`/`std` traits
//!       (never `core::fmt::{Debug, Display}` as they use `&dyn`)
//! - [ ] generics on the trait
//! - [ ] associated types
//! - [ ] supertraits
//!     - [ ] upcasting `Ref<dyn Subtrait>` to `Ref<dyn Supertrait>`
//! - [ ] `Pin<&mut self>` and similar non-reference object-safe receivers
//! - [ ] `where` bounds on the trait
//! - [ ] `where Self: Sized` methods (and appropriate exclusion from the vtable)
//!     - [ ] non-lifetime generics on methods
//!     - [ ] non-lifetime `where` bounds on methods
//!     - [ ] An attribute to manually exclude a method from a vtable, necessary for bounds
//!           including subtraits or aliases of `Sized`
//! - [ ] An `tinydyn(inline_vtable[ = "all"])` attribute to force inlining of the vtable into the
//!       wide pointer. This would require the metadata type to always be carried in the trait.
//! - [ ] Put `Ref` vtables inline even if `RefMut` won't. Ex: 1 `&self` and 1 `&mut self` method.
//! - [ ] UI tests to ensure proper rejection and error message quality
//!
//! ### Implementing on foreign traits
//!
//! Implementing on foreign traits is not yet supported, and is an ergonomic and safety challenge
//! to get right, especially with regards to default methods.
//! As a workaround, you can create a local helper trait that contains the desired methods and
//! blanket implements for all `T: TargetTrait`.
//! This functionality might be added by tinydyn in the future, or a better solution like defining
//! custom local vtables for foreign traits.
//! Traits with supertraits that wish to use this version of `tinydyn` have a similar workaround.
//!
//! ## Design
//!
//! See the [README] for how this library is designed and works.
//!
//! [README]: https://github.com/kupiakos/tinydyn/blob/main/README.md
#![no_std]
#![warn(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

use core::marker::PhantomData;

use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

// This contains types that may change at any time without a breaking library change,
// but must be exposed so macros can reference them.
// If you're naming types from here yourself, beware.
#[doc(hidden)]
#[path = "private.rs"]
pub mod __private;

/// Marks a local trait as tinydyn-aware, letting it be used inside of [`Ref`] and [`RefMut`].
///
/// This implements [`DynTrait`] and [`PlainDyn`] for the targeted trait object.
/// This defines an alternate smaller vtable layout that erases layout and drop information.
///
/// While you *can* use tinydyn-aware traits as regular `dyn Trait` trait objects, it's not
/// recommended as it creates two vtables.
///
/// # Example
///
/// ```ignore
/// use tinydyn::{self, tinydyn};
///
/// #[tinydyn]
/// trait Foo {
///     fn blah(&self) -> i32;
///     fn blue(&self) -> i32 { 10 }
/// }
///
/// impl Foo for i32 {
///     fn blah(&self) -> i32 { *self + 1 }
/// }
///
/// // Use `dyn Foo` to reference the trait `Foo` even though it never creates the
/// // regular vtable for `dyn Foo`.
/// let x: tinydyn::Ref<dyn Foo> = Ref::new(&15);
/// assert_eq!(x.blah(), 16);
/// assert_eq!(x.blue(), 10);
/// ```
pub use tinydyn_derive::tinydyn;

use __private::DynTarget;

/// Wraps `T` with the local newtype associated with this tinydyn trait.
///
/// See [`PlainDyn::LocalNewtype`] for more information.
type LocalWrap<Trait, T> = <<Trait as DynTrait>::Plain as PlainDyn>::LocalNewtype<T>;

/// A shared reference to a tinydyn trait object.
///
/// `Ref<dyn Trait>` can call the `&self` methods of `Trait` through its `Deref` impl.
/// It can also be freely cloned and copied like a `&dyn Trait`.
///
/// Prefer passing this around rather than calling `deref` and passing around that reference
/// - that would create a double pointer.
#[repr(C)]
pub struct Ref<'a, Trait: ?Sized + DynTrait> {
    inner: DynPtr<'a, Trait>,
    _lifetime: PhantomData<&'a Trait>,
}

impl<'a, Trait: ?Sized + DynTrait + 'a> Copy for Ref<'a, Trait> {}
impl<'a, Trait: ?Sized + DynTrait + 'a> Clone for Ref<'a, Trait> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _lifetime: PhantomData,
        }
    }
}

impl<'a, Trait: ?Sized + DynTrait + 'a> Deref for Ref<'a, Trait> {
    type Target = DynTarget<Trait>;

    /// It's not recommended to hold onto the result of this `deref`, as it creates a
    /// double reference.
    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}

impl<'a, Trait: ?Sized + DynTrait> Ref<'a, Trait> {
    /// Upcasts this `&U` into a `Ref<dyn Trait>` so long as `U: Trait`.
    ///
    /// This builds a tinydyn vtable and references it in the returned `Ref`.
    pub fn new<U>(r: &'a U) -> Self
    where
        LocalWrap<Trait, U>: Implements<Trait>,
    {
        let data = NonNull::from(r).cast();
        let meta = <LocalWrap<Trait, U> as BuildDynMeta<Trait::Plain>>::metadata();
        let inner = unsafe { DynPtr::new(data, meta) };
        Self {
            inner,
            _lifetime: PhantomData,
        }
    }

    unsafe fn from_inner(inner: DynPtr<'a, Trait>) -> Self {
        Self {
            inner,
            _lifetime: PhantomData,
        }
    }
}

impl<'a, Trait: ?Sized + DynTrait + Send + 'a> Ref<'a, Trait> {
    /// Removes the `Send` bound from `Trait`, if any.
    pub fn remove_send(self) -> Ref<'a, Trait::RemoveSend> {
        unsafe { Ref::from_inner(self.inner.remove_send()) }
    }
}

impl<'a, Trait: ?Sized + DynTrait + Sync + 'a> Ref<'a, Trait> {
    /// Removes the `Sync` bound from `Trait`, if any.
    pub fn remove_sync(self) -> Ref<'a, Trait::RemoveSync> {
        unsafe { Ref::from_inner(self.inner.remove_sync()) }
    }
}

// TODO: consider allowing the `Ref` lifetime to be smaller here
impl<'a, Trait: ?Sized + DynTrait> From<RefMut<'a, Trait>> for Ref<'a, Trait> {
    fn from(value: RefMut<'a, Trait>) -> Self {
        Ref {
            inner: value.inner,
            _lifetime: PhantomData,
        }
    }
}

unsafe impl<'a, Trait> Send for Ref<'a, Trait>
where
    Trait: ?Sized + DynTrait,
    &'a Trait: Send,
{
}

unsafe impl<'a, Trait> Sync for Ref<'a, Trait>
where
    Trait: ?Sized + DynTrait,
    &'a Trait: Sync,
{
}

/// A mutable reference to a tinydyn trait object.
///
/// `RefMut<dyn Trait>` can call the `&self` and `&mut self` methods of `Trait` through its
/// `Deref` impl.
///
/// Like `&mut dyn Trait`, this cannot be cloned or copied. It can, however, be [reborrowed].
///
/// Prefer passing this around rather than calling `deref_mut` and passing around that reference
/// - that would create a double pointer.
///
/// [reborrowed]: RefMut::as_mut
#[repr(transparent)]
pub struct RefMut<'a, Trait: ?Sized + DynTrait> {
    inner: DynPtr<'a, Trait>,
    _lifetime: PhantomData<&'a mut Trait>,
}

impl<'a, Trait: ?Sized + DynTrait + 'a> Deref for RefMut<'a, Trait> {
    type Target = DynTarget<Trait>;

    /// It's not recommended to hold onto the result of this `deref`, as it creates a
    /// double reference.
    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}

impl<'a, Trait: ?Sized + DynTrait + 'a> DerefMut for RefMut<'a, Trait> {
    /// It's not recommended to hold onto the result of this `deref`, as it creates a
    /// double reference.
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.deref_mut()
    }
}

impl<'a, Trait: ?Sized + DynTrait> RefMut<'a, Trait> {
    /// Upcasts this `&mut U` into a `RefMut<dyn Trait>` so long as `U: Trait`.
    ///
    /// This builds a tinydyn vtable and references it in the returned `RefMut`.
    pub fn new<U>(r: &'a mut U) -> Self
    where
        LocalWrap<Trait, U>: Implements<Trait>,
    {
        let data = NonNull::from(r).cast();
        let meta = <LocalWrap<Trait, U> as BuildDynMeta<Trait::Plain>>::metadata();
        let inner = unsafe { DynPtr::new(data, meta) };
        Self {
            inner,
            _lifetime: PhantomData,
        }
    }
}

impl<'a, Trait: ?Sized + DynTrait + 'a> RefMut<'a, Trait> {
    /// Constructs a `RefMut` from its raw inner pointer.
    ///
    /// # Safety
    /// The `inner` pointer must be safe to mutate through.
    unsafe fn from_inner(inner: DynPtr<'a, Trait>) -> Self {
        Self {
            inner,
            _lifetime: PhantomData,
        }
    }

    /// Reborrow as a shared `Ref` with a smaller lifetime.
    ///
    /// Since a `RefMut` isn't `Copy`, this is needed to pass to a function expecting a `Ref` and
    /// regain access to the underlying `RefMut` after it's done.
    pub fn as_ref<'b>(&'b self) -> Ref<'b, Trait> {
        unsafe { Ref::from_inner(self.inner) }
    }

    /// Reborrow as a `RefMut` with a smaller lifetime.
    ///
    /// Since a `RefMut` isn't `Copy`, this is needed to pass to a function expecting a `RefMut`
    /// and regain access to the underlying `RefMut` after it's done.
    pub fn as_mut<'b>(&'b mut self) -> RefMut<'b, Trait>
    where
        'a: 'b,
    {
        unsafe { RefMut::from_inner(self.inner) }
    }

    /// Downcasts this [`RefMut`] into a raw pointer of concrete type.
    ///
    /// The pointer can accessed soundly for the lifetime of `&mut self`
    /// so long as `self` was constructed from a `&mut T`.
    pub fn as_downcast_ptr<T>(&mut self) -> NonNull<T> {
        self.inner.data.cast()
    }

    /// Gets the pointer metadata for this trait object.
    pub fn metadata(&self) -> <Trait::Plain as PlainDyn>::Metadata {
        self.inner.meta
    }
}

impl<'a, Trait: ?Sized + DynTrait + Send + 'a> RefMut<'a, Trait> {
    /// Removes the `Send` bound from `Trait`, if any.
    pub fn remove_send(self) -> RefMut<'a, Trait::RemoveSend> {
        unsafe { RefMut::from_inner(self.inner.remove_send()) }
    }
}

impl<'a, Trait: ?Sized + DynTrait + Sync + 'a> RefMut<'a, Trait> {
    /// Removes the `Sync` bound from `Trait`, if any.
    pub fn remove_sync(self) -> RefMut<'a, Trait::RemoveSync> {
        unsafe { RefMut::from_inner(self.inner.remove_sync()) }
    }
}

unsafe impl<'a, Trait> Send for RefMut<'a, Trait>
where
    Trait: ?Sized + DynTrait,
    &'a mut Trait: Send,
{
}

unsafe impl<'a, Trait> Sync for RefMut<'a, Trait>
where
    Trait: ?Sized + DynTrait,
    &'a mut Trait: Sync,
{
}

/// The shared inner pointer of [`Ref`] and [`RefMut`].
pub(crate) struct DynPtr<'a, Trait: ?Sized + DynTrait> {
    data: NonNull<()>,
    meta: <Trait::Plain as PlainDyn>::Metadata,
    _lifetime: PhantomData<&'a ()>,
}

unsafe impl<'a, Trait> Send for DynPtr<'a, Trait>
where
    Trait: ?Sized + DynTrait + Send,
    <Trait::Plain as PlainDyn>::Metadata: Send,
{
}

unsafe impl<'a, Trait> Sync for DynPtr<'a, Trait>
where
    Trait: ?Sized + DynTrait + Sync,
    <Trait::Plain as PlainDyn>::Metadata: Sync,
{
}

impl<'a, Trait: ?Sized + DynTrait + 'a> Copy for DynPtr<'a, Trait> {}
impl<'a, Trait: ?Sized + DynTrait + 'a> Clone for DynPtr<'a, Trait> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            meta: self.meta.clone(),
            _lifetime: PhantomData,
        }
    }
}

impl<'a, Trait: ?Sized + DynTrait + 'a> DynPtr<'a, Trait> {
    pub(crate) unsafe fn new(
        data: NonNull<()>,
        meta: <Trait::Plain as PlainDyn>::Metadata,
    ) -> Self {
        Self {
            data,
            meta,
            _lifetime: PhantomData,
        }
    }
}

impl<'a, Trait: ?Sized + DynTrait + Send + 'a> DynPtr<'a, Trait> {
    /// Removes the `Send` bound from `Trait`, if any.
    pub fn remove_send(self) -> DynPtr<'a, Trait::RemoveSend> {
        DynPtr {
            data: self.data,
            meta: self.meta,
            _lifetime: PhantomData,
        }
    }
}

impl<'a, Trait: ?Sized + DynTrait + Sync + 'a> DynPtr<'a, Trait> {
    /// Removes the `Sync` bound from `Trait`, if any.
    pub fn remove_sync(self) -> DynPtr<'a, Trait::RemoveSync> {
        DynPtr {
            data: self.data,
            meta: self.meta,
            _lifetime: PhantomData,
        }
    }
}

impl<'a, Trait: ?Sized + DynTrait + 'a> Deref for DynPtr<'a, Trait> {
    type Target = DynTarget<Trait>;

    fn deref(&self) -> &Self::Target {
        DynTarget::new_ref(self)
    }
}

impl<'a, Trait: ?Sized + DynTrait + 'a> DerefMut for DynPtr<'a, Trait> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        DynTarget::new_mut(self)
    }
}

/// A trait object supported by `tinydyn`.
///
/// # Safety
/// Must be implemented on `dyn Trait` objects without extra bounds, including lifetimes.
pub unsafe trait PlainDyn: DynTrait<Plain = Self> {
    /// The metadata carried alongside a `Ref` and `RefMut` used to call trait functions.
    type Metadata: Copy;

    /// The vtable duplicated for each combination of trait and concrete type.
    type StaticVTable: Copy;

    /// A local generic transparent newtype.
    ///
    /// Used to work around the [coherence] rules by tying in a local newtype.
    ///
    /// - Blanket impl for a foreign trait:
    ///   [`BuildDynMeta`] is implemented for `LocalNewtype<T>` where `T` implements this trait.
    ///
    /// # Safety for implementers
    ///
    /// - This must be `#[repr(transparent)]` over `T`.
    /// - This must not add any additional functionality, excluding that added by `tinydyn`.
    ///
    /// [coherence]: https://github.com/rust-lang/rfcs/blob/master/text/2451-re-rebalancing-coherence.md
    type LocalNewtype<T>;
}

/// A trait object that works with `tinydyn`, including any extra bounds.
///
/// `dyn Trait` erases that a trait object implements `Send` or `Sync`, so
/// Rust allows a concrete type that implements either of those traits to
/// cast to a `dyn Trait [+ Send][+ Sync]` object.
///
/// `tinydyn` itself doesn't depend on `Send` or `Sync` the important code.
///
/// # Safety
/// The associated types must be correct as described.
pub unsafe trait DynTrait {
    /// The trait object without any extra bounds.
    type Plain: PlainDyn + ?Sized;

    /// The trait object without the `+ Send` bound. If it has none, this must be `Self`.
    type RemoveSend: DynTrait<Plain = Self::Plain> + ?Sized;

    /// The trait object without the `+ Sync` bound. If it has none, this must be `Self`.
    type RemoveSync: DynTrait<Plain = Self::Plain> + ?Sized;
}

/// Builds the tinydyn trait metadata for a given type.
///
/// This metadata is shared by `dyn Trait [+ Send] [+ Sync]`.
///
/// Implemented for `LocalNewtype<T>` where `T` implements the `Trait`.
pub unsafe trait BuildDynMeta<Trait>
where
    Self: Sized,
    Trait: PlainDyn + ?Sized,
{
    /// The contents of the vtable for this type.
    /// If the metadata is a function pointer, this is unused.
    const STATIC_VTABLE: Trait::StaticVTable;

    /// Gets the pointer metadata necessary to call trait methods.
    fn metadata() -> Trait::Metadata;
}

/// Types that could be cast to the given `Trait` trait object.
///
/// Implemented by `LocalNewtype<T>` where `T` implements the `Trait`.
pub unsafe trait Implements<Trait>
where
    Self: BuildDynMeta<Trait::Plain>,
    Trait: DynTrait + ?Sized,
{
}
