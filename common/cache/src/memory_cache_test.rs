// Copyright 2021 Datafuse Labs.
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

use std::borrow::Borrow;

use crate::memory_cache::LruCache;
use crate::memory_cache::Meter;

#[test]
fn test_put_and_get() {
    let mut cache = LruCache::new(2);
    cache.insert(1, 10);
    cache.insert(2, 20);
    assert_eq!(cache.get_mut(&1), Some(&mut 10));
    assert_eq!(cache.get_mut(&2), Some(&mut 20));
    assert_eq!(cache.len(), 2);
    assert_eq!(cache.size(), 2);
}

#[test]
fn test_put_update() {
    let mut cache = LruCache::new(1);
    cache.insert("1", 10);
    cache.insert("1", 19);
    assert_eq!(cache.get_mut("1"), Some(&mut 19));
    assert_eq!(cache.len(), 1);
}

#[test]
fn test_contains_key() {
    let mut cache = LruCache::new(1);
    cache.insert("1", 10);
    assert!(cache.contains_key("1"));
}

#[test]
fn test_expire_lru() {
    let mut cache = LruCache::new(2);
    cache.insert("foo1", "bar1");
    cache.insert("foo2", "bar2");
    cache.insert("foo3", "bar3");
    assert!(cache.get_mut("foo1").is_none());
    cache.insert("foo2", "bar2update");
    cache.insert("foo4", "bar4");
    assert!(cache.get_mut("foo3").is_none());
}

#[test]
fn test_pop() {
    let mut cache = LruCache::new(2);
    cache.insert(1, 10);
    cache.insert(2, 20);
    assert_eq!(cache.len(), 2);
    let opt1 = cache.remove(&1);
    assert!(opt1.is_some());
    assert_eq!(opt1.unwrap(), 10);
    assert!(cache.get_mut(&1).is_none());
    assert_eq!(cache.len(), 1);
}

#[test]
fn test_change_capacity() {
    let mut cache = LruCache::new(2);
    assert_eq!(cache.capacity(), 2);
    cache.insert(1, 10);
    cache.insert(2, 20);
    cache.set_capacity(1);
    assert!(cache.get_mut(&1).is_none());
    assert_eq!(cache.capacity(), 1);
}

#[test]
fn test_debug() {
    let mut cache = LruCache::new(3);
    cache.insert(1, 10);
    cache.insert(2, 20);
    cache.insert(3, 30);
    assert_eq!(format!("{:?}", cache), "{3: 30, 2: 20, 1: 10}");
    cache.insert(2, 22);
    assert_eq!(format!("{:?}", cache), "{2: 22, 3: 30, 1: 10}");
    cache.insert(6, 60);
    assert_eq!(format!("{:?}", cache), "{6: 60, 2: 22, 3: 30}");
    cache.get_mut(&3);
    assert_eq!(format!("{:?}", cache), "{3: 30, 6: 60, 2: 22}");
    cache.set_capacity(2);
    assert_eq!(format!("{:?}", cache), "{3: 30, 6: 60}");
}

#[test]
fn test_remove() {
    let mut cache = LruCache::new(3);
    cache.insert(1, 10);
    cache.insert(2, 20);
    cache.insert(3, 30);
    cache.insert(4, 40);
    cache.insert(5, 50);
    cache.remove(&3);
    cache.remove(&4);
    assert!(cache.get_mut(&3).is_none());
    assert!(cache.get_mut(&4).is_none());
    cache.insert(6, 60);
    cache.insert(7, 70);
    cache.insert(8, 80);
    assert!(cache.get_mut(&5).is_none());
    assert_eq!(cache.get_mut(&6), Some(&mut 60));
    assert_eq!(cache.get_mut(&7), Some(&mut 70));
    assert_eq!(cache.get_mut(&8), Some(&mut 80));
}

#[test]
fn test_clear() {
    let mut cache = LruCache::new(2);
    cache.insert(1, 10);
    cache.insert(2, 20);
    cache.clear();
    assert!(cache.get_mut(&1).is_none());
    assert!(cache.get_mut(&2).is_none());
    assert_eq!(format!("{:?}", cache), "{}");
}

#[test]
fn test_iter() {
    let mut cache = LruCache::new(3);
    cache.insert(1, 10);
    cache.insert(2, 20);
    cache.insert(3, 30);
    cache.insert(4, 40);
    cache.insert(5, 50);
    assert_eq!(cache.iter().collect::<Vec<_>>(), [
        (&3, &30),
        (&4, &40),
        (&5, &50)
    ]);
    assert_eq!(cache.iter_mut().collect::<Vec<_>>(), [
        (&3, &mut 30),
        (&4, &mut 40),
        (&5, &mut 50)
    ]);
    assert_eq!(cache.iter().rev().collect::<Vec<_>>(), [
        (&5, &50),
        (&4, &40),
        (&3, &30)
    ]);
    assert_eq!(cache.iter_mut().rev().collect::<Vec<_>>(), [
        (&5, &mut 50),
        (&4, &mut 40),
        (&3, &mut 30)
    ]);
}

struct VecLen;

impl<K, T> Meter<K, Vec<T>> for VecLen {
    type Measure = usize;
    fn measure<Q: ?Sized>(&self, _: &Q, v: &Vec<T>) -> usize
    where K: Borrow<Q> {
        v.len()
    }
}

#[test]
fn test_metered_cache() {
    let mut cache = LruCache::with_meter(5, VecLen);
    cache.insert("foo1", vec![1, 2]);
    assert_eq!(cache.size(), 2);
    cache.insert("foo2", vec![3, 4]);
    cache.insert("foo3", vec![5, 6]);
    assert_eq!(cache.size(), 4);
    assert!(!cache.contains_key("foo1"));
    cache.insert("foo2", vec![7, 8]);
    cache.insert("foo4", vec![9, 10]);
    assert_eq!(cache.size(), 4);
    assert!(!cache.contains_key("foo3"));
    assert_eq!(cache.get("foo2"), Some(&vec![7, 8]));
}

#[test]
fn test_metered_cache_reinsert_larger() {
    let mut cache = LruCache::with_meter(5, VecLen);
    cache.insert("foo1", vec![1, 2]);
    cache.insert("foo2", vec![3, 4]);
    assert_eq!(cache.size(), 4);
    cache.insert("foo2", vec![5, 6, 7, 8]);
    assert_eq!(cache.size(), 4);
    assert!(!cache.contains_key("foo1"));
}

#[test]
fn test_metered_cache_oversize() {
    let mut cache = LruCache::with_meter(2, VecLen);
    cache.insert("foo1", vec![1, 2]);
    cache.insert("foo2", vec![3, 4, 5, 6]);
    assert_eq!(cache.size(), 0);
    assert!(!cache.contains_key("foo1"));
    assert!(!cache.contains_key("foo2"));
}

#[cfg(feature = "heapsize")]
#[test]
fn test_heapsize_cache() {
    use super::HeapSize;

    let mut cache = LruCache::<&str, (u8, u8, u8), _, _>::with_meter(8, HeapSize);
    cache.insert("foo1", (1, 2, 3));
    cache.insert("foo2", (4, 5, 6));
    cache.insert("foo3", (7, 8, 9));
    assert!(!cache.contains_key("foo1"));
    cache.insert("foo2", (10, 11, 12));
    cache.insert("foo4", (13, 14, 15));
    assert!(!cache.contains_key("foo3"));
}
