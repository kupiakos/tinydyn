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
    fn blah(&self) -> i32;
    fn blue(&self) -> i32 {
        10
    }
}

impl Foo for i32 {
    fn blah(&self) -> i32 {
        *self + 1
    }
}

impl Foo for u16 {
    fn blah(&self) -> i32 {
        i32::from(*self) + 10
    }

    fn blue(&self) -> i32 {
        i32::from(*self) - 10
    }
}

#[test]
fn default_method() {
    let mut x: tinydyn::Ref<dyn Foo> = Ref::new(&15i32);
    assert_eq!(x.blah(), 16);
    assert_eq!(x.blue(), 10);
    x = Ref::new(&0u16);
    assert_eq!(x.blah(), 10);
    assert_eq!(x.blue(), -10);
}
