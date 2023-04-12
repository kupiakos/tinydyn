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

#[tinydyn]
trait Foo {
    fn method_lt_generic<'a>(&self, x: &'a i32) -> &'a i32;
    fn bound_lt_generic<'a, 'b>(&'a self, x: &'b i32) -> &'b i32
    where
        'a: 'b;
    fn as_bar(&self) -> Bar;
}

#[derive(Clone, Debug, PartialEq)]
struct Bar(i32);
impl Foo for Bar {
    fn method_lt_generic<'a>(&self, x: &'a i32) -> &'a i32 {
        if self.0 == 0 {
            x
        } else {
            &100
        }
    }

    fn bound_lt_generic<'a, 'b>(&'a self, x: &'b i32) -> &'b i32
    where
        'a: 'b,
    {
        if self.0 < *x {
            &self.0
        } else {
            x
        }
    }

    fn as_bar(&self) -> Bar {
        self.clone()
    }
}

#[test]
fn method_lifetime_generics() {
    let x = Bar(0);
    let y: Ref<dyn Foo> = Ref::new(&x);
    assert_eq!(*y.method_lt_generic(&10), 10);
    assert_eq!(*y.bound_lt_generic(&1), 0);
    assert_eq!(y.as_bar(), Bar(0));
}
