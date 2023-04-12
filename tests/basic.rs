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

use std::cell::Cell;

use tinydyn::{tinydyn, DynTrait, Ref};

#[tinydyn]
trait IntCell {
    fn get(&self) -> i32;
    fn set(&self, value: i32);
}

impl IntCell for Cell<i32> {
    fn get(&self) -> i32 {
        self.get()
    }

    fn set(&self, value: i32) {
        self.set(value)
    }
}

#[repr(C)]
struct IntCellRef<'a>(&'a Cell<i32>);

impl IntCell for IntCellRef<'_> {
    fn get(&self) -> i32 {
        self.0.get()
    }

    fn set(&self, value: i32) {
        self.0.set(value)
    }
}

struct Dispatcher<'a, Trait: ?Sized + DynTrait> {
    // client: &'a dyn CheckReady,
    client: tinydyn::Ref<'a, Trait>,
}

impl<'a> Dispatcher<'a, dyn IntCell> {
    fn new(client: tinydyn::Ref<'a, dyn IntCell>) -> Self {
        Self { client }
    }
    // fn set_client(&mut self, client: &'a dyn CheckReady) {
    fn set_client(&mut self, client: tinydyn::Ref<'a, dyn IntCell>) {
        self.client = client;
    }

    fn dispatch(&self, value: i32) {
        self.client.set(value);
    }
}

#[test]
fn all_scalar() {
    let cell1 = Cell::new(1);
    let cell2 = Cell::new(2);
    let cell2 = IntCellRef(&cell2);
    let mut dispatcher = Dispatcher::new(Ref::new(&cell1));
    dispatcher.dispatch(10);
    assert_eq!(cell1.get(), 10);
    dispatcher.set_client(Ref::new(&cell2));
    dispatcher.dispatch(20);
    assert_eq!(cell1.get(), 10);
    assert_eq!(cell2.get(), 20);
}
