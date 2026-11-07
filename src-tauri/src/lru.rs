//! A small size-capped cache that forgets the least recently used entry.
//! Eviction scans for the oldest entry, which is fine at the few hundred
//! entries the app's caches hold.

use std::collections::HashMap;
use std::hash::Hash;

/// Holds at most `CAP` entries.
pub struct Lru<K, V, const CAP: usize> {
    tick: u64,
    map: HashMap<K, (V, u64)>,
}

impl<K, V, const CAP: usize> Default for Lru<K, V, CAP> {
    fn default() -> Self {
        Self {
            tick: 0,
            map: HashMap::new(),
        }
    }
}

impl<K: Hash + Eq + Clone, V, const CAP: usize> Lru<K, V, CAP> {

    /// The cached value, marking it as just used.
    pub fn get(&mut self, key: &K) -> Option<&V> {
        self.tick += 1;
        let tick = self.tick;
        self.map.get_mut(key).map(|(v, used)| {
            *used = tick;
            &*v
        })
    }

    /// Cache `value`, first forgetting the least recently used entry when
    /// full.
    pub fn insert(&mut self, key: K, value: V) {
        self.tick += 1;
        if !self.map.contains_key(&key)
            && self.map.len() >= CAP.max(1)
            && let Some(oldest) = self
                .map
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
        {
            self.map.remove(&oldest);
        }
        self.map.insert(key, (value, self.tick));
    }
}

#[cfg(test)]
mod tests {
    use super::Lru;

    #[test]
    fn evicts_least_recently_used() {
        let mut c = Lru::<_, _, 2>::default();
        c.insert("a", 1);
        c.insert("b", 2);
        assert_eq!(c.get(&"a"), Some(&1)); // a is now the fresher one
        c.insert("c", 3);
        assert_eq!(c.map.len(), 2);
        assert_eq!(c.get(&"b"), None, "b was least recently used");
        assert_eq!(c.get(&"a"), Some(&1));
        assert_eq!(c.get(&"c"), Some(&3));
    }

    #[test]
    fn replacing_a_key_does_not_evict() {
        let mut c = Lru::<_, _, 2>::default();
        c.insert("a", 1);
        c.insert("b", 2);
        c.insert("a", 10);
        assert_eq!(c.map.len(), 2);
        assert_eq!(c.get(&"a"), Some(&10));
        assert_eq!(c.get(&"b"), Some(&2));
    }
}
