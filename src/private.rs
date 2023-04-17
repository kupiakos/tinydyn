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

//! This contains types that may change at any time without a breaking library change,
//! but must be exposed so macros can reference them.
//! If you're naming types from here yourself, beware.

use core::marker::PhantomData;
use core::ptr::NonNull;

use crate::DynPtr;
use crate::Implements;
use crate::{BuildDynMeta, DynTrait, PlainDyn};

/// A type-erased pointer used to represent `&self` and `&mut self`.
#[repr(transparent)]
pub struct SelfPtr<TraitPtr>(NonNull<()>, PhantomData<TraitPtr>);

impl<Trait: ?Sized + PlainDyn> SelfPtr<*const Trait> {
    pub(crate) fn new_ref(self_: NonNull<()>) -> Self {
        Self(self_, PhantomData)
    }

    /// `self` must have been constructed from a `&T`.
    pub unsafe fn downcast<'a, T>(self) -> &'a T
    where
        Trait::LocalNewtype<T>: BuildDynMeta<Trait>,
    {
        unsafe { self.0.cast().as_ref() }
    }

    pub fn upcast<Super>(self) -> SelfPtr<*const Super>
    where
        Trait: Implements<Super>,
        Super: ?Sized + PlainDyn,
    {
        SelfPtr(self.0, PhantomData)
    }
}

impl<Trait: ?Sized + PlainDyn> SelfPtr<*mut Trait> {
    pub(crate) fn new_mut(self_: NonNull<()>) -> Self {
        Self(self_, PhantomData)
    }

    /// Unerases the pointer by downcasting to a concrete type.
    ///
    /// This does no runtime check.
    ///
    /// # Safety
    /// - `self` must have been constructed from a `&mut T`.
    pub unsafe fn downcast_mut<'a, T>(self) -> &'a mut T
    where
        Trait::LocalNewtype<T>: BuildDynMeta<Trait>,
    {
        unsafe { self.0.cast().as_mut() }
    }
}

/// This acts as an unsized `Deref` target for [`Ref`] and [`RefMut`].
///
/// This serves two purposes:
///
/// - `DynTarget` is not `Sized`, so it's stopped from calling `where Self: Sized` trait
///    functions at compile time.
/// - You cannot soundly swap an unowned unsized object, so the lifetime of `Self` doesn't have to
///   reflect the lifetime of the trait object.
///
/// Avoid naming this type. You probably shouldn't be storing references to this directly,
/// but might be necessary for isolated cases. For those, prefer writing `&(impl Trait + ?Sized)`.
///
/// ```compile_fail
// #[doc = doctest_use_private!(Ref)]
/// # use tinydyn::{tinydyn_trait, Ref};
/// #[tinydyn_trait]
/// trait Foo {
///     fn trait_object_safe(&self) {}
///     fn trait_object_unsafe(self) where Self: Sized {}
/// }
/// impl Foo for () {}
/// let x: Ref<dyn Foo> = Ref::new(&());
/// // Cannot compile because `DynTarget<dyn Read>` is not `Sized`!
/// let y = x.by_ref();
/// ```
///
/// [`Ref`]: super::Ref
/// [`RefMut`]: super::RefMut
// TODO(kupiakos): try to hide this from the public interface entirely and require going through
//                 `<Ref<dyn Trait> as Deref>::Target`, to allow that `Target` to change without
//                 causing a library breaking change.
#[repr(C)]
pub struct DynTarget<Trait: ?Sized + DynTrait> {
    // The lifetime of `ptr` cannot escape this `DynTarget`
    ptr: DynPtr<'static, Trait>,
    _phantom: PhantomData<Trait>,

    /// A slice whose only purpose is to carry a length.
    /// This length is the dynamic metadata, function pointer or static vtable.
    ///
    /// Since it is a 1-aligned ZST, it does not affect the layout of the type.
    /// So, since `DynTarget` is `repr(C)`, it has the same size/align as `data`.
    make_unsized: [()],
}

impl<Trait: ?Sized + DynTrait> DynTarget<Trait> {
    #[inline(always)]
    pub(crate) fn new_ref<'a>(ptr: &'a DynPtr<'a, Trait>) -> &'a Self {
        let wide_slice: *const [DynPtr<'a, Trait>] = core::ptr::slice_from_raw_parts(ptr, 0);
        // SAFETY:
        // - The pointer cast preserves the pointer metadata of a 0 slice length.
        // - `make_unsized` has size 0 and alignment 1, meaning `DynTarget<Trait>` has the same
        //   layout as `DynPtr<'_, Trait>`.
        // - The data portion of `wide_slice` retains provenance for a `DynPtr<'_, Trait>`
        //   as an intermediate reference with smaller provenance was never formed.
        // - The lifetime of `ptr` is never exposed, and there is no legal way to swap two
        //   unowned unsized types.
        unsafe { &*(wide_slice as *const Self) }
    }

    #[inline(always)]
    pub(crate) fn new_mut<'a, 'b: 'a>(ptr: &'a mut DynPtr<'b, Trait>) -> &'a mut Self {
        let wide_slice: *mut [DynPtr<'_, Trait>] = core::ptr::slice_from_raw_parts_mut(ptr, 0);
        // SAFETY:
        // - See `Self::new`
        unsafe { &mut *(wide_slice as *mut Self) }
    }

    #[inline(always)]
    pub fn self_ref(self_: &Self) -> SelfPtr<*const Trait::Plain> {
        SelfPtr::new_ref(self_.ptr.data)
    }

    #[inline(always)]
    pub fn self_mut(self_: &mut Self) -> SelfPtr<*mut Trait::Plain> {
        SelfPtr::new_mut(self_.ptr.data)
    }

    /// Get the dyn metadata for this wide pointer.
    pub fn meta(self_: &Self) -> <Trait::Plain as PlainDyn>::Metadata {
        self_.ptr.meta
    }
}

/// The [`PlainDyn::StaticVTable`] for traits without a vtable.
///
/// This is for traits that have one function defined, and so can store a function pointer
/// instead of a pointer to a vtable.
#[derive(Clone, Copy)]
pub struct InlineVTable;

/// Unsafely `transmute` from `Src` to `Dst` with a transmute check at runtime,
/// and not compile time.
///
/// The panic *should* always be compiled out. If it isn't, something's wrong.
#[inline(always)]
pub unsafe fn runtime_layout_verified_transmute<Src, Dst>(src: Src) -> Dst {
    assert!(
        core::alloc::Layout::new::<Src>() == core::alloc::Layout::new::<Dst>(),
        "Bare argument layout mismatch. This indicates a bug in tinydyn."
    );
    let src_manual_drop = core::mem::ManuallyDrop::new(src);
    let dst =
        unsafe { core::mem::transmute_copy::<core::mem::ManuallyDrop<Src>, Dst>(&src_manual_drop) };
    dst
}
