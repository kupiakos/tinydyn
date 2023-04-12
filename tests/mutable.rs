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

use tinydyn::{tinydyn, RefMut};

#[tinydyn]
trait Do {
    fn mutate(&mut self, value: i32);
}

impl Do for i32 {
    fn mutate(&mut self, value: i32) {
        *self = value;
    }
}

#[test]
fn mutable() {
    let mut x = 10;
    let mut z: RefMut<dyn Do> = RefMut::new(&mut x);
    z.mutate(25);
    assert_eq!(x, 25);
    let mut y = 30;
    z = RefMut::new(&mut y);
    z.mutate(35);
    assert_eq!(y, 35);
}
