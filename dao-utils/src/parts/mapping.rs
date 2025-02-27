use std::{
    collections::{hash_map::DefaultHasher, BTreeMap},
    hash::{Hash, Hasher},
    marker::PhantomData,
    sync::Mutex,
};

use casper_contract::{
    contract_api::{runtime, storage},
    unwrap_or_revert::UnwrapOrRevert,
};
use casper_types::{
    bytesrepr::{FromBytes, ToBytes},
    CLTyped,
    Key,
    URef,
};
use lazy_static::lazy_static;

use crate::{casper_env::to_dictionary_key, Error, Instanced};

/// Data structure for storing key-value pairs.
///
/// It's is a wrapper on top of Casper's dictionary.
/// The main difference is that Mapping returns default value, if the value doesn't exists
/// and it stores dictionary's URef for later use.
pub struct Mapping<K, V> {
    name: String,
    key_ty: PhantomData<K>,
    value_ty: PhantomData<V>,
}

lazy_static! {
    static ref SEEDS: Mutex<BTreeMap<String, URef>> = Mutex::new(BTreeMap::new());
}

impl<K: ToBytes + CLTyped, V: ToBytes + FromBytes + CLTyped> Mapping<K, V> {
    /// Create new Mapping instance.
    pub fn new(name: String) -> Self {
        Mapping {
            name,
            key_ty: PhantomData::<K>::default(),
            value_ty: PhantomData::<V>::default(),
        }
    }

    /// Read `key` from the storage or return None
    pub fn get(&self, key: &K) -> Option<V> {
        self.get_or_none(key)
    }

    /// Read `key` from the storage or revert if the key stores no value.
    pub fn get_or_revert(&self, key: &K) -> V {
        self.get_or_none(key)
            .unwrap_or_revert_with(Error::ValueNotAvailable)
    }

    /// Read `key` from the storage or return none.
    pub fn get_or_none(&self, key: &K) -> Option<V> {
        storage::dictionary_get(self.get_uref(), &to_dictionary_key(key))
            .unwrap_or_revert_with(Error::DictionaryStorageError)
    }

    /// Set `value` under `key` to the storage. It overrides by default.
    pub fn set(&self, key: &K, value: V) {
        storage::dictionary_put(self.get_uref(), &to_dictionary_key(key), value);
    }

    /// Return the named key path to the dictionarie's URef.
    pub fn path(&self) -> &str {
        &self.name
    }

    fn get_uref(&self) -> URef {
        let mut seeds = SEEDS.lock().unwrap();
        let seed = seeds.get(&self.name);
        match seed {
            Some(seed) => *seed,
            None => {
                let key: Key = match runtime::get_key(&self.name) {
                    Some(key) => key,
                    None => {
                        storage::new_dictionary(&self.name)
                            .unwrap_or_revert_with(Error::DictionaryStorageError);
                        runtime::get_key(&self.name)
                            .unwrap_or_revert_with(Error::KeyValueStorageError)
                    }
                };
                let seed: URef = *key.as_uref().unwrap_or_revert_with(Error::VMInternalError);
                seeds.insert(self.name.clone(), seed);
                seed
            }
        }
    }
}

impl<K: ToBytes + CLTyped, V: ToBytes + FromBytes + CLTyped> Instanced for Mapping<K, V> {
    fn instance(namespace: &str) -> Self {
        namespace.into()
    }
}

impl<K: ToBytes + CLTyped, V: ToBytes + FromBytes + CLTyped> From<&str> for Mapping<K, V> {
    fn from(name: &str) -> Self {
        Mapping::new(name.to_string())
    }
}

/// Data structure for storing index-value pairs.
///
/// Similar to [`Mapping`], but a key is always `u32`.
pub struct IndexedMapping<V> {
    mapping: Mapping<u32, Option<V>>,
    index: Index,
}

impl<V: ToBytes + FromBytes + CLTyped + Hash> IndexedMapping<V> {
    /// Creates new IndexedMapping instance.
    pub fn new(name: String) -> Self {
        IndexedMapping {
            mapping: Mapping::new(name.clone()),
            index: Index::new(name),
        }
    }

    /// Set `value` under `index` to the storage. It overrides by default.
    pub fn set(&self, index: u32, value: V) {
        self.index.set(index, &value);
        self.mapping.set(&index, Some(value));
    }

    /// Reads `index` from the storage or return None
    pub fn get(&self, index: u32) -> Option<V> {
        self.mapping.get(&index).unwrap_or(None)
    }

    /// Replaces the 'value` with `None`. Returns a tuple (success, altered_index).
    pub fn remove(&self, value: V) -> (bool, u32) {
        if let Some(item_index) = self.index.get(&value) {
            if self.mapping.get(&item_index).is_some() {
                self.index.unset(&value);
                self.mapping.set(&item_index, None);
                return (true, item_index);
            }
        }
        (false, 0)
    }

    /// Set `None` under `index` to the storage. It overrides by default.
    pub fn unset(&self, index: u32) {
        self.mapping.set(&index, None);
    }

    /// Returns the index of the `value` or None if does not exists.
    pub fn index_of(&self, value: &V) -> Option<u32> {
        self.index.get(value)
    }

    /// Checks if the `value` is stored in the collection.
    pub fn contains(&self, value: &V) -> bool {
        matches!(self.index_of(value), Some(_))
    }

    /// Returns the named key path to the dictionary's URef.
    pub fn path(&self) -> &str {
        self.mapping.path()
    }
}

impl<T: FromBytes + ToBytes + CLTyped> Instanced for IndexedMapping<T> {
    fn instance(namespace: &str) -> Self {
        Self {
            mapping: Instanced::instance(&format!("{}:{}", namespace, "mapping")),
            index: Instanced::instance(&format!("{}:{}", namespace, "index")),
        }
    }
}

/// Data structure for storing key-value pairs.
///
/// Similar to [`Mapping`], but a key is complex - is a (K, u32) tuple.
/// The related values have the same first part of the key and a different index.
pub struct VecMapping<K, V> {
    mapping: Mapping<(K, u32), V>,
    lengths: Mapping<K, u32>,
}

impl<K: CLTyped + FromBytes + ToBytes + Hash + Clone, V: ToBytes + FromBytes + CLTyped>
    VecMapping<K, V>
{
    /// Creates new VecMapping instance.
    pub fn new(name: String) -> Self {
        VecMapping {
            mapping: Mapping::new(name.clone()),
            lengths: Mapping::new(format!("{}_length", name)),
        }
    }

    /// Replaces the `value` under (`key`, `at`) key.
    pub fn replace(&self, key: K, at: u32, value: V) -> Result<(), Error> {
        let length = self.lengths.get(&key).unwrap_or(0);
        if at >= length {
            return Err(Error::MappingIndexDoesNotExist);
        }
        self.mapping.set(&(key, at), value);
        Ok(())
    }

    /// Reads `key` at the given position from the storage or return None
    pub fn get(&self, key: K, at: u32) -> Option<V> {
        self.mapping.get(&(key, at))
    }

    /// Aggregates all the values stored under (`key`, n).
    pub fn get_all(&self, key: K) -> Vec<V> {
        let length = self.lengths.get(&key).unwrap_or(0);
        let mut result = Vec::new();
        for i in 0..length {
            if let Some(value) = self.get(key.clone(), i) {
                result.push(value);
            }
        }

        result
    }

    /// Reads `key` at the given position from the storage or return None
    pub fn get_or_none(&self, key: K, at: u32) -> Option<V> {
        self.mapping.get_or_none(&(key, at))
    }

    /// Read `key` at the given position from the storage or revert if the key stores no value.
    pub fn get_or_revert(&self, key: K, at: u32) -> V {
        self.mapping.get_or_revert(&(key, at))
    }

    /// Sets `value` under the next index of `key` to the storage. It overrides by default.
    pub fn add(&self, key: K, value: V) {
        let length = self.lengths.get(&key).unwrap_or(0);
        self.lengths.set(&key, length + 1);
        self.mapping.set(&(key, length), value);
    }

    /// Number of items stored under `key`.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self, key: K) -> u32 {
        self.lengths.get(&key).unwrap_or(0)
    }
}

impl<K: ToBytes + CLTyped, V: FromBytes + ToBytes + CLTyped> Instanced for VecMapping<K, V> {
    fn instance(namespace: &str) -> Self {
        Self {
            mapping: Instanced::instance(&format!("{}:{}", namespace, "mapping")),
            lengths: Instanced::instance(&format!("{}:{}", namespace, "lengths")),
        }
    }
}

pub struct Index {
    index: Mapping<u64, Option<u32>>,
}

impl Index {
    pub fn new(name: String) -> Self {
        Index {
            index: Mapping::new(format!("{}{}", name, "_idx")),
        }
    }

    pub fn set<V: ToBytes + FromBytes + CLTyped + Hash>(&self, index: u32, value: &V) {
        let value_hash = Index::calculate_hash(value);
        self.index.set(&value_hash, Some(index));
    }

    pub fn unset<V: ToBytes + FromBytes + CLTyped + Hash>(&self, value: &V) {
        let value_hash = Index::calculate_hash(value);
        self.index.set(&value_hash, None);
    }

    pub fn get<V: ToBytes + FromBytes + CLTyped + Hash>(&self, value: &V) -> Option<u32> {
        let value_hash = Index::calculate_hash(value);
        self.index.get(&value_hash).unwrap_or(None)
    }

    fn calculate_hash<V: ToBytes + FromBytes + CLTyped + Hash>(value: &V) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }
}

impl Instanced for Index {
    fn instance(namespace: &str) -> Self {
        Self {
            index: Instanced::instance(&format!("{}:{}", namespace, "index")),
        }
    }
}
