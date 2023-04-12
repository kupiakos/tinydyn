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

//! Tiny dynamic dispatch for embedded use.
//!
//! [`Ref<dyn Trait>`] and [`RefMut<dyn Trait>`] wrap a pointer and metadata necessary to call
//! trait methods, and [`Deref`] into a _tinydyn trait object_ that implements the `Trait`.
//!
//! Traits must currently opt-in by annotating with [`tinydyn`].
//! This defines an alternate, lighter weight vtable, and if the trait has one method, eliminates
//! it entirely by putting the function pointer inline.
//! This does not affect normal behavior of the trait, and can still be made into a `dyn Trait`.
//! This, however, would be wasteful.
//!
//! # Example
//! ```ignore  TODO figure out why this is broken
//! # use tinydyn::{tinydyn, Ref};
//! #[tinydyn]
//! trait Foo {
//!     fn blah(&self) -> i32;
//!     fn blue(&self) -> i32 { 10 }
//! }
//! impl Foo for i32 {
//!     fn blah(&self) -> i32 { *self + 1 }
//! }
//!
//! // Like upcasting to `&dyn Foo`, but with a tinydyn vtable.
//! let x: Ref<dyn Foo> = Ref::new(&15);
//! assert_eq!(x.blah(), 16);
//! assert_eq!(x.blue(), 10);
//! ```
//!
//! # Space Savings
//!
//! TODO: numbers on a real large project
//!
//! For every trait and concrete type which upcasts into that trait, Rust creates a new vtable.
//! Each vtable includes 3 extra words of layout and drop info. These aren't needed, so tinydyn's
//! custom vtables do not include them.
//!
//! In addition, tinydyn places the vtable inline in `Ref[Mut]` if it has only one method.
//! This saves a dereference when making the virtual call as well as removing the need for a static
//! vtable to allocate - truly as zero-cost as dynamic dispatch can get!
//!
//! # Design
//!
//! ## Features supported
//! - `&self` and `&mut self` methods
//! - `+ Send` and `+ Sync` trait objects
//! - lifetime `where` bounds on methods
//! - lifetime generics on methods
//!
//! ## Features not yet implemented, but planned
//! - custom vtables/implementing on foreign traits
//! - implementations for common `core`/`std` traits
//!   (not `core::fmt::{Debug, Display}` as they use `&dyn`)
//! - generics on the trait
//! - associated types
//! - supertraits
//! - `Pin<&mut self>` and similar non-reference object-safe receivers
//! - `where` bounds on the trait
//! - `where Self: Sized` methods (and appropriate exclusion from the vtable)
//!     - non-lifetime generics on methods
//!     - non-lifetime `where` bounds on methods
//!
//! ### Foreign traits
//!
//! Implementing on foreign traits is not yet supported, and is an ergonomic and safety challenge.
//! As a workaround, you can create a local helper trait that contains the desired methods and
//! blanket implements for all `T: TargetTrait`.
//! This functionality might be added by tinydyn in the future, or a better solution.
//! Supertraits have a similar workaround.
//!
//! ## Double Pointer
//! For safety reasons, the unsized trait object that [`Ref`]/ [`RefMut`] deref into is a
//! pointer to the trait object, creating a double pointer to the object. So, while you _can_ turn
//! them into a `&(impl Trait + ?Sized)`, that will be marginally larger code size if not optimized.
//!
//! ## Why can't `dyn Trait` be made smaller as an optimization?
//!
//! `dyn Trait` requires a vtable be generated for each combination of
//! concrete type and trait. This vtable includes 3 words of information that can't
//! be removed in order for Rust to work with it:
//! - The type's size. Returned by [`core::mem::size_of_val`] and used to drop.
//! - The type's alignment, needed to calculate the field offset of a `dyn Trait` located at the
//!   end of a struct.
//!   Returned by [`core::mem::align_of_val`] and used to drop.
//! - The drop glue, used to drop and equivalent to `ptr::drop_in_place::<Concrete>`.
//!
//! Most of this is about `Drop` glue, only relevant for use in a `Box`, as well as the already
//! stabilized size/align getters for the type for unsafe code to manipulate it (soundly).
//!
//! In theory, `rustc` could identify that a trait object's size, align, and drop glue are never
//! accessed throughout the whole program and remove them from the vtable, possibly even inlining
//! the vtable as tinydyn does. However, rustc is averse to global analysis, preferring to leave
//! this to LLVM; and LLVM doesn't know how trait object vtables are formatted.
//!
//! These are requirements tinydyn doesn't have to uphold. It doesn't have a `Box`.
//!
//! ## A trait object that doesn't know its size
//!
//! Since tinydyn trait objects don't know the size or alignment of what they point to, no
//! reference to the concrete type can be made while the type is erased.
//!
//! So, in order for the tinydyn trait object to implement a trait, the implementer itself has to
//! have an erased pointer type. If that pointer type is sized, however, that comes with its own set
//! of issues. You can [swap](`core::mem::swap`) two `Sized` references, and trait object-unsafe
//! functions marked with `where Self: Sized` are now available to call, even though there's no
//! possible implementation.
//!
//! tinydyn trait objects do this with a specific design:
//! - They're primarily referenced through the [`Ref`] and [`RefMut`] types, which hold the data
//! pointer and metadata needed to call trait methods with no overhead.
//! - These don't implement the trait, but `Deref` into a `!Sized` wrapper object that does,
//! called the *dyn wrapper*.
//! - The dyn wrapper holds same pointer as the `Ref[Mut]`,
//! so the `Deref` creates a double reference to avoid creating a direct reference to the target.
//! - The deref wrapper object is discouraged from being used through reference like trait objects
//! normally are. Not only does it have an inaccurate `size_of_val` and `align_of_val`, it is
//! a double pointer and is more expensive to use directly.
//! - The vtable-calling functions are marked `#[inline(always)]` so the double pointer created
//! when calling trait methods is detected as unnecessary and optimized away by LLVM.
//!
//! This fake layout and double dereference is, in the end, a necessary design decision for
//! soundness.
#![warn(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

use core::marker::PhantomData;

use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;

#[doc(hidden)]
#[path = "private.rs"]
pub mod __private;

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
/// [reborrowed]: RefMut::as_mut
#[repr(transparent)]
pub struct RefMut<'a, Trait: ?Sized + DynTrait> {
    inner: DynPtr<'a, Trait>,
    _lifetime: PhantomData<&'a mut Trait>,
}

impl<'a, Trait: ?Sized + DynTrait + 'a> Deref for RefMut<'a, Trait> {
    // type Target = LocalWrap<Trait, DynPtr<'a, Trait>>;
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
