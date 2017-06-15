use super::*;
use std::sync::Arc;

pub struct Db {
    next_id: u64,
    idents: IdentMap,
    store: Arc<KVStore + 'static>,
    eav: Index<Record, EAVT>,
    ave: Index<Record, AVET>,
    aev: Index<Record, AEVT>,
}

impl Db {
    pub fn new(store: Arc<KVStore>) -> Result<Db> {
        let contents: DbContents = store.get_contents()?;

        let node_store = btree::NodeStore { backing_store: store.clone() };
        let mut db = Db {
            next_id: contents.next_id,
            store: store,
            idents: contents.idents,
            eav: Index::new(contents.eav, node_store.clone(), EAVT)?,
            ave: Index::new(contents.ave, node_store.clone(), AVET)?,
            aev: Index::new(contents.aev, node_store, AEVT)?,
        };

        db.idents = db.idents.add("db:ident".to_string(), Entity(1));

        if db.next_id == 0 {
            // Bootstrap some attributes we need to run transactions,
            // because they need to reference one another.

            // Initial transaction entity
            db.add(Record::new(Entity(0),
                               Entity(2),
                               Value::Timestamp(UTC::now()),
                               Entity(0)));

            // Entity for the db:ident attribute
            db.add(Record::new(Entity(1),
                               Entity(1),
                               Value::Ident("db:ident".into()),
                               Entity(0)));

            // Entity for the db:txInstant attribute
            db.add(Record::new(Entity(2),
                               Entity(1),
                               Value::Ident("db:txInstant".into()),
                               Entity(0)));
        }

        Ok(db)
    }

    fn get_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn add(&mut self, record: Record) {
        if record.entity.0 >= self.next_id {
            self.next_id = record.entity.0 + 1;
        }

        self.eav = self.eav.insert(record.clone()).unwrap();
        self.ave = self.ave.insert(record.clone()).unwrap();
        self.aev = self.aev.insert(record.clone()).unwrap();

        // If the record has a db:ident, we need to add it to the ident map.
        if record.attribute == self.idents.get_entity("db:ident".to_string()).unwrap() {
            match record.value {
                Value::Ident(s) => self.idents = self.idents.add(s.clone(), record.entity),
                _ => unimplemented!(), // FIXME: type error
            };
        }
    }

    /// Saves the db metadata (index root nodes, entity ID state) to
    /// storage, when implemented by the storage backend (i.e. when
    /// not using in-memory storage).
    fn save_contents(&self) -> Result<()> {
        let contents = DbContents {
            next_id: self.next_id,
            idents: self.idents.clone(),
            eav: self.eav.root_ref.clone(),
            aev: self.aev.root_ref.clone(),
            ave: self.ave.root_ref.clone(),
        };

        self.store.set_contents(&contents)?;
        Ok(())
    }

    fn records_matching(&self, clause: &Clause, binding: &Binding) -> Result<Vec<Record>> {
        let expanded = clause.substitute(binding)?;
        match expanded {
            // ?e a v => use the ave index
            Clause {
                entity: Term::Unbound(_),
                attribute: Term::Bound(a),
                value: Term::Bound(v),
            } => {
                match self.idents.get_entity(a) {
                    Some(attr) => {
                        let range_start = Record::new(Entity(0), attr, v.clone(), Entity(0));
                        Ok(self.ave
                               .iter_range_from(range_start..)
                               .unwrap()
                               .map(|res| res.unwrap())
                               .take_while(|rec| rec.attribute == attr && rec.value == v)
                               .collect())
                    }
                    _ => return Err("invalid attribute".into()),
                }
            }
            // // e a ?v => use the eav index
            Clause {
                entity: Term::Bound(e),
                attribute: Term::Bound(a),
                value: Term::Unbound(_),
            } => {
                match self.idents.get_entity(a) {
                    Some(attr) => {
                        // Value::String("") is the lowest-sorted value
                        let range_start = Record::new(e, attr, Value::String("".into()), Entity(0));
                        Ok(self.eav
                               .iter_range_from(range_start..)
                               .unwrap()
                               .map(|res| res.unwrap())
                               .take_while(|rec| rec.entity == e && rec.attribute == attr)
                               .collect())
                    }
                    _ => return Err("invalid attribute".into()),
                }
            }
            // FIXME: Implement other optimized index use cases? (multiple unknowns? refs?)
            // Fallthrough case: just scan the EAV index. Correct but slow.
            _ => {
                Ok(self.eav
                    .iter()
                    .map(|f| f.unwrap()) // FIXME this is not safe :D
                    .filter(|f| unify(&binding, &self.idents, &clause, &f).is_some())
                    .collect())
            }
        }
    }

    pub fn transact(&mut self, tx: Tx) -> Result<TxReport> {
        let mut new_entities = vec![];
        let tx_entity = Entity(self.get_id());
        let attr = self.idents.get_entity("db:txInstant".to_string()).unwrap();
        self.add(Record::new(tx_entity, attr, Value::Timestamp(UTC::now()), tx_entity));
        for item in tx.items {
            match item {
                TxItem::Addition(f) => {
                    let attr = self.idents
                        .get_entity(f.attribute)
                        .ok_or("invalid attribute".to_string())?;
                    self.add(Record::new(f.entity, attr, f.value, tx_entity))
                }
                TxItem::NewEntity(ht) => {
                    let entity = Entity(self.get_id());
                    for (k, v) in ht {
                        let attr = self.idents
                            .get_entity(k)
                            .ok_or("invalid attribute".to_string())?;
                        self.add(Record::new(entity, attr, v, tx_entity))
                    }
                    new_entities.push(entity);
                }
                // TODO Implement retractions
                _ => unimplemented!(),
            }
        }
        self.save_contents()?;
        Ok(TxReport { new_entities })
    }

    pub fn query(&self, query: &Query) -> Result<QueryResult> {
        // TODO: automatically bind ?tx in queries
        let mut bindings = vec![HashMap::new()];

        for clause in &query.clauses {
            let mut new_bindings = vec![];

            for binding in bindings {
                for record in self.records_matching(clause, &binding)? {
                    match unify(&binding, &self.idents, clause, &record) {
                        Some(new_info) => {
                            let mut new_env = binding.clone();
                            new_env.extend(new_info);
                            new_bindings.push(new_env)
                        }
                        _ => continue,
                    }
                }
            }

            bindings = new_bindings;
        }

        for binding in bindings.iter_mut() {
            *binding = binding
                .iter()
                .filter(|&(k, _)| query.find.contains(k))
                .map(|(var, value)| (var.clone(), value.clone()))
                .collect();
        }

        Ok(QueryResult(query.find.clone(), bindings))
    }
}

/// Attempts to unify a new record and a clause with existing
/// bindings.  If bound fields in the clause match the record, then
/// any fields in the record which match an unbound clause will be
/// bound in the returned binding.  If bound fields in the clause
/// conflict with fields in the record, unification fails.
fn unify(env: &Binding, idents: &IdentMap, clause: &Clause, record: &Record) -> Option<Binding> {
    let mut new_info: Binding = Default::default();

    match clause.entity {
        Term::Bound(ref e) => {
            if *e != record.entity {
                return None;
            }
        }
        Term::Unbound(ref var) => {
            match env.get(var) {
                Some(e) => {
                    if *e != Value::Entity(record.entity) {
                        return None;
                    }
                }
                _ => {
                    new_info.insert(var.clone(), Value::Entity(record.entity));
                }
            }
        }
    }

    match clause.attribute {
        Term::Bound(ref a) => {
            // The query will use an ident to refer to the attribute, but we need the
            // actual attribute entity.
            match idents.get_entity(a.to_owned()) {
                Some(e) => {
                    if e != record.attribute {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        Term::Unbound(ref var) => {
            match env.get(var) {
                Some(e) => {
                    if *e != Value::Entity(record.attribute) {
                        return None;
                    }
                }
                _ => {
                    new_info.insert(var.clone(), Value::Entity(record.attribute));
                }
            }
        }
    }

    match clause.value {
        Term::Bound(ref v) => {
            if *v != record.value {
                return None;
            }
        }
        Term::Unbound(ref var) => {
            match env.get(var) {
                Some(e) => {
                    if *e != record.value {
                        return None;
                    }
                }
                _ => {
                    new_info.insert(var.clone(), record.value.clone());
                }
            }
        }
    }

    Some(new_info)
}

/// A structure designed to be stored in the backing store that enables
/// a process to locate the indexes, tx log, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbContents {
    pub next_id: u64,
    pub idents: IdentMap,
    pub eav: String,
    pub ave: String,
    pub aev: String,
}

pub fn store_from_uri(uri: &str) -> Result<Arc<KVStore>> {
    match &uri.split("//").collect::<Vec<_>>()[..] {
        &["logos:mem:", _] => Ok(Arc::new(HeapStore::new()) as Arc<KVStore>),
        &["logos:sqlite:", path] => {
            let sqlite_store = SqliteStore::new(path)?;
            Ok(Arc::new(sqlite_store) as Arc<KVStore>)
        }
        &["logos:cass:", url] => {
            let cass_store = CassandraStore::new(url)?;
            Ok(Arc::new(cass_store) as Arc<KVStore>)
        }
        _ => Err("Invalid uri".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate test;
    use self::test::{Bencher, black_box};

    use std::iter;
    use std::sync::Arc;

    use super::*;
    use backends::mem::HeapStore;
    use db::Db;

    fn expect_query_result(query: &Query, expected: QueryResult) {
        let db = test_db();
        let result = db.query(query).unwrap();
        assert_eq!(expected, result);
    }

    fn test_db() -> Db {
        let store = HeapStore::new();
        let mut db = Db::new(Arc::new(store)).unwrap();
        let records = vec![
            Fact::new(Entity(0), "name", "Bob"),
            Fact::new(Entity(1), "name", "John"),
            Fact::new(Entity(2), "Hello", "World"),
            Fact::new(Entity(1), "parent", Entity(0)),
        ];

        parse_tx("{db:ident name} {db:ident parent} {db:ident Hello}")
            .map(|tx| db.transact(tx))
            .unwrap()
            .unwrap();

        db.transact(Tx {
                          items: records
                              .iter()
                              .map(|x| TxItem::Addition(x.clone()))
                              .collect(),
                      })
            .unwrap();

        db
    }



    #[test]
    fn test_query_unknown_entity() {
        // find ?a where (?a name "Bob")
        expect_query_result(&parse_query("find ?a where (?a name \"Bob\")").unwrap(),
                            QueryResult(vec![Var::new("a")],
                                        vec![
            iter::once((Var::new("a"), Value::Entity(Entity(0))))
                .collect(),
        ]));
    }

    #[test]
    fn test_query_unknown_value() {
        // find ?a where (0 name ?a)
        expect_query_result(&parse_query("find ?a where (0 name ?a)").unwrap(),
                            QueryResult(vec![Var::new("a")],
                                        vec![
            iter::once((Var::new("a"),
                        Value::String("Bob".into())))
                    .collect(),
        ]));

    }

    // // It's inconvenient to test this because we don't have a ref to the db in
    // // the current setup, and we don't know the entity id of `name` offhand.
    // #[test]
    // fn test_query_unknown_attribute() {
    //     // find ?a where (1 ?a "John")
    //     expect_query_result(&parse_query("find ?a where (1 ?a \"John\")").unwrap(),
    //                         QueryResult(vec![Var::new("a")],
    //                                     vec![
    //         iter::once((Var::new("a"),
    //                     Value::String("name".into())))
    //                 .collect(),
    //     ]));
    // }

    #[test]
    fn test_query_multiple_results() {
        // find ?a ?b where (?a name ?b)
        expect_query_result(&parse_query("find ?a ?b where (?a name ?b)").unwrap(),
                            QueryResult(vec![Var::new("a"), Var::new("b")],
                                        vec![
            vec![
                (Var::new("a"), Value::Entity(Entity(0))),
                (Var::new("b"), Value::String("Bob".into())),
            ]
                    .into_iter()
                    .collect(),
            vec![
                (Var::new("a"), Value::Entity(Entity(1))),
                (Var::new("b"), Value::String("John".into())),
            ]
                    .into_iter()
                    .collect(),
        ]));
    }

    #[test]
    fn test_query_explicit_join() {
        // find ?b where (?a name Bob) (?b parent ?a)
        expect_query_result(&parse_query("find ?b where (?a name \"Bob\") (?b parent ?a)")
                                 .unwrap(),
                            QueryResult(vec![Var::new("b")],
                                        vec![
            iter::once((Var::new("b"), Value::Entity(Entity(1))))
                .collect(),
        ]));
    }

    #[test]
    fn test_query_implicit_join() {
        // find ?c where (?a name Bob) (?b name ?c) (?b parent ?a)
        expect_query_result(&parse_query("find ?c where (?a name \"Bob\") (?b name ?c) (?b parent ?a)")
                    .unwrap(),
               QueryResult(vec![Var::new("c")],
                           vec![
            iter::once((Var::new("c"), Value::String("John".into())))
                .collect(),
        ]));
    }

    #[test]
    fn test_type_mismatch() {
        let db = test_db();
        let q = &parse_query("find ?e ?n where (?e name ?n) (?n name \"hi\")").unwrap();
        assert_equal(db.query(&q), Err("type mismatch".to_string()))
    }

    #[bench]
    // Parse + run a query on a small db
    fn parse_bench(b: &mut Bencher) {
        // the implicit join query
        let input = black_box(r#"find ?c where (?a name "Bob") (?b name ?c) (?b parent ?a)"#);

        b.iter(|| parse_query(input).unwrap());
    }

    #[bench]
    // Parse + run a query on a small db
    fn run_bench(b: &mut Bencher) {
        // the implicit join query
        let input = black_box(r#"find ?c where (?a name "Bob") (?b name ?c) (?b parent ?a)"#);
        let query = parse_query(input).unwrap();
        let db = test_db();

        b.iter(|| db.query(&query));
    }

    #[bench]
    fn bench_add(b: &mut Bencher) {
        let store = HeapStore::new();
        let mut db = Db::new(Arc::new(store)).unwrap();
        parse_tx("{db:ident blah}")
            .map(|tx| db.transact(tx))
            .unwrap()
            .unwrap();

        let a = db.idents.get_entity("blah".to_string()).unwrap();

        let mut e = 0;

        b.iter(|| {
                   let entity = Entity(e);
                   e += 1;

                   db.add(Record::new(entity, a, Value::Entity(entity), Entity(0)));
               });
    }

    fn test_db_large() -> Db {
        let store = HeapStore::new();
        let mut db = Db::new(Arc::new(store)).unwrap();
        let n = 10_000_000;

        for i in 0..n {
            let a = if i % 23 < 10 {
                "name".to_string()
            } else {
                "Hello".to_string()
            };

            let v = if i % 1123 == 0 { "Bob" } else { "Rob" };

            let attr = db.idents.get_entity(a).unwrap();
            db.add(Record::new(Entity(i), attr, v, Entity(0)));
        }

        db
    }


    #[test]
    fn test_records_matching() {
        let matching = test_db()
            .records_matching(&Clause::new(Term::Unbound("e".into()),
                                           Term::Bound("name".into()),
                                           Term::Bound(Value::String("Bob".into()))),
                              &Binding::default())
            .unwrap();
        assert_eq!(matching.len(), 1);
        let rec = &matching[0];
        assert_eq!(rec.entity, Entity(0));
        assert_eq!(rec.value, Value::String("Bob".into()));
    }
    // Don't run on 'cargo test', only 'cargo bench'
    #[cfg(not(debug_assertions))]
    #[bench]
    fn large_db_simple(b: &mut Bencher) {
        let query = black_box(parse_query(r#"find ?a where (?a name "Bob")"#).unwrap());
        let db = test_db_large();

        b.iter(|| db.query(&query));
    }
}