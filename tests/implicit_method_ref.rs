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

use tinydyn::{tinydyn, Ref};

#[derive(Clone, Copy)]
struct Wrap<'a>(&'a i32);

#[tinydyn]
trait GetIntRef {
    fn get(&self) -> &i32;
    fn get_wrap(&self) -> Wrap<'_>;
    fn get_arg_ref(&self, x: &i32) -> Option<&i32>;
}

impl GetIntRef for Wrap<'_> {
    fn get(&self) -> &i32 {
        &self.0
    }

    fn get_wrap(&self) -> Wrap<'_> {
        *self
    }

    fn get_arg_ref(&self, x: &i32) -> Option<&i32> {
        if *x == 0 {
            None
        } else {
            Some(&self.0)
        }
    }
}

#[test]
fn implicit_method_ref() {
    let x = Wrap(&10);
    let y: Ref<dyn GetIntRef> = Ref::new(&x);
    assert_eq!(*y.get(), 10);
    assert_eq!(*y.get_wrap().0, 10);
    assert_eq!(y.get_arg_ref(&20), Some(&10));
}
