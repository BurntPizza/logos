pub mod sqlite;
pub mod mem;

use btree::IndexNode;
use ident::IdentMap;

/// Abstracts over various backends; all that's required for a Logos
/// backend is the ability to add a key, retrieve a key, and
/// atomically set/get the DbContents.
pub trait KVStore : Clone {
    type Item;

    // Add an item to the database
    fn add(&self, value: IndexNode<Self::Item>) -> Result<String, String>;
    fn set_contents(&self, contents: &DbContents) -> Result<(), String>;
    // Used to retrieve references to indices from possibly-persistent storage
    fn get_contents(&self) -> Result<DbContents, String>;
    fn get(&self, key: &str) -> Result<IndexNode<Self::Item>, String>;
}

/// A structure designed to be stored in the index that enables
/// a process to locate the indexes, tx log, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbContents {
    pub next_id: u64,
    pub idents: IdentMap,
    pub eav: String,
    pub ave: String,
    pub aev: String,
}