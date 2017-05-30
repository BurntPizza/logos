
#![feature(collections_range)]
#![feature(conservative_impl_trait)]

#[macro_use]
extern crate itertools;

#[macro_use]
extern crate combine;

#[cfg(test)]
#[macro_use]
extern crate lazy_static;

use itertools::*;

use std::fmt::{self, Display, Formatter};
use std::collections::HashMap;
use std::collections::BTreeSet;
use std::iter;

pub mod parser;
mod print_table;

pub use parser::*;

// A database is just a log of facts. Facts are (entity, attribute, value) triples.
// Attributes and values are both just strings. There are no transactions or histories.

#[derive(Debug, PartialEq)]
pub struct QueryResult(Vec<Var>, Vec<HashMap<Var, Value>>);

impl Display for QueryResult {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let col_names = self.0.iter().map(|v| &*v.name);

        let aligns = iter::repeat(print_table::Alignment::Center);
        let rows = self.1
            .iter()
            .map(|row_ht| self.0.iter().map(|var| format!("{}", row_ht[var])).collect_vec());

        writeln!(f,
                 "{}",
                 print_table::debug_table("Result", col_names, aligns, rows))
    }
}

#[derive(Debug, PartialEq, Eq, Clone, PartialOrd, Ord)]
pub enum Value {
    String(String),
    Entity(Entity),
}

impl Display for Value {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f,
               "{}",
               match *self {
                   Value::Entity(e) => format!("{}", e.0),
                   Value::String(ref s) => format!("{:?}", s),
               })
    }
}

impl<T: Into<String>> From<T> for Value {
    fn from(x: T) -> Self {
        Value::String(x.into())
    }
}

impl From<Entity> for Value {
    fn from(x: Entity) -> Self {
        Value::Entity(x.into())
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
enum Term<T> {
    Bound(T),
    Unbound(Var),
}

// A free [logic] variable
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub struct Var {
    name: String,
}

impl Var {
    fn new<T: Into<String>>(name: T) -> Var {
        Var { name: name.into() }
    }
}

impl<T: Into<String>> From<T> for Var {
    fn from(x: T) -> Self {
        Var { name: x.into() }
    }
}

// A query looks like `find ?var where (?var <attribute> <value>)`
#[derive(Debug, PartialEq)]
pub struct Query {
    find: Vec<Var>,
    clauses: Vec<Clause>,
}

impl Query {
    fn new(find: Vec<Var>, clauses: Vec<Clause>) -> Query {
        Query {
            find: find,
            clauses: clauses,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Tx {
    items: Vec<TxItem>,
}

#[derive(Debug, PartialEq, Eq)]
enum TxItem {
    Addition(Fact),
    Retraction(Fact),
    NewEntity(HashMap<String, Value>),
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, PartialOrd, Ord)]
pub struct Entity(u64);

#[derive(Debug, PartialEq, Eq)]
pub struct Clause {
    entity: Term<Entity>,
    attribute: Term<String>,
    value: Term<Value>,
}

impl Clause {
    fn new(e: Term<Entity>, a: Term<String>, v: Term<Value>) -> Clause {
        Clause {
            entity: e,
            attribute: a,
            value: v,
        }
    }
}

pub trait Database {
    fn add(&mut self, fact: Fact);
    fn facts_matching(&self, clause: &Clause, binding: &Binding) -> Vec<&Fact>;

    fn transact(&mut self, tx: Tx) {
        for item in tx.items {
            match item {
                TxItem::Addition(f) => self.add(f),
                // TODO Implement retractions + new entities
                _ => unimplemented!(),
            }
        }
    }

    fn query(&self, query: Query) -> QueryResult {
        let mut bindings = vec![HashMap::new()];

        for clause in &query.clauses {
            let mut new_bindings = vec![];

            for binding in bindings {
                for fact in self.facts_matching(clause, &binding) {
                    match unify(&binding, clause, &fact) {
                        Ok(new_env) => new_bindings.push(new_env),
                        _ => continue,
                    }
                }
            }

            bindings = new_bindings;
        }

        let result = bindings.into_iter()
            .map(|solution| {
                solution.into_iter()
                    .filter(|&(ref k, _)| query.find.contains(&k))
                    .collect()
            })
            .collect();

        QueryResult(query.find, result)
    }
}

// The Fact struct represents a fact in the database.
// The derived ordering is used by the EAV index; other
// indices use orderings provided by wrapper structs.
#[derive(Debug, PartialEq, Eq, Ord, PartialOrd, Clone)]
pub struct Fact {
    entity: Entity,
    attribute: String,
    value: Value,
}

impl Fact {
    fn new<A: Into<String>, V: Into<Value>>(e: Entity, a: A, v: V) -> Fact {
        Fact {
            entity: e,
            attribute: a.into(),
            value: v.into(),
        }
    }
}

// Fact wrappers provide ordering for indexes.
#[derive(PartialEq, Eq, Debug)]
struct AVE(Fact);

impl PartialOrd for AVE {
    fn partial_cmp(&self, other: &AVE) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AVE {
    fn cmp(&self, other: &AVE) -> std::cmp::Ordering {
        self.0
            .attribute
            .cmp(&other.0.attribute)
            .then(self.0.value.cmp(&other.0.value))
            .then(self.0.entity.cmp(&other.0.entity))
    }
}

#[derive(PartialEq, Eq, Debug)]
struct AEV(Fact);

impl PartialOrd for AEV {
    fn partial_cmp(&self, other: &AEV) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AEV {
    fn cmp(&self, other: &AEV) -> std::cmp::Ordering {
        self.0
            .attribute
            .cmp(&other.0.attribute)
            .then(self.0.entity.cmp(&other.0.entity))
            .then(self.0.value.cmp(&other.0.value))
    }
}

type Binding = HashMap<Var, Value>;

impl Clause {
    fn substitute(&self, env: &Binding) -> Clause {
        let entity = match &self.entity {
            &Term::Bound(_) => self.entity.clone(),
            &Term::Unbound(ref var) => {
                if let Some(val) = env.get(&var) {
                    match *val {
                        Value::Entity(e) => Term::Bound(e),
                        _ => unimplemented!(),
                    }
                } else {
                    self.entity.clone()
                }
            }
        };

        let attribute = match &self.attribute {
            &Term::Bound(_) => self.attribute.clone(),
            &Term::Unbound(ref var) => {
                if let Some(val) = env.get(&var) {
                    match val {
                        &Value::String(ref s) => Term::Bound(s.to_owned()),
                        _ => unimplemented!(),
                    }
                } else {
                    self.attribute.clone()
                }
            }
        };

        let value = match &self.value {
            &Term::Bound(_) => self.value.clone(),
            &Term::Unbound(ref var) => {
                if let Some(val) = env.get(&var) {
                    Term::Bound(val.clone())
                } else {
                    self.value.clone()
                }
            }
        };

        Clause::new(entity, attribute, value)
    }
}

#[derive(Debug, Default)]
pub struct InMemoryLog {
    eav: BTreeSet<Fact>,
    ave: BTreeSet<AVE>,
    aev: BTreeSet<AEV>,
}

use std::collections::range::RangeArgument;
use std::collections::Bound;

impl RangeArgument<Fact> for Fact {
    fn start(&self) -> Bound<&Fact> {
        Bound::Included(&self)
    }

    fn end(&self) -> Bound<&Fact> {
        Bound::Unbounded
    }
}

impl RangeArgument<AEV> for AEV {
    fn start(&self) -> Bound<&AEV> {
        Bound::Included(&self)
    }

    fn end(&self) -> Bound<&AEV> {
        Bound::Unbounded
    }
}

impl RangeArgument<AVE> for AVE {
    fn start(&self) -> Bound<&AVE> {
        Bound::Included(&self)
    }

    fn end(&self) -> Bound<&AVE> {
        Bound::Unbounded
    }
}

impl InMemoryLog {
    pub fn new() -> InMemoryLog {
        InMemoryLog::default()
    }
}

impl IntoIterator for InMemoryLog {
    type Item = Fact;
    type IntoIter = <std::collections::BTreeSet<Fact> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.eav.into_iter()
    }
}


impl Database for InMemoryLog {
    fn add(&mut self, fact: Fact) {
        self.eav.insert(fact.clone());
        self.ave.insert(AVE(fact.clone()));
        self.aev.insert(AEV(fact.clone()));
    }

    fn facts_matching(&self, clause: &Clause, binding: &Binding) -> Vec<&Fact> {
        let expanded = clause.substitute(binding);
        match expanded {
            // ?e a v => use the ave index
            Clause { entity: Term::Unbound(_),
                     attribute: Term::Bound(a),
                     value: Term::Bound(v) } => {
                let range_start = Fact::new(Entity(0), a.clone(), v.clone());
                self.ave
                    .range(AVE(range_start))
                    .map(|ave| &ave.0)
                    .take_while(|f| f.attribute == *a && f.value == v)
                    .collect()
            }
            // e a ?v => use the eav index
            Clause { entity: Term::Bound(e),
                     attribute: Term::Bound(a),
                     value: Term::Unbound(_) } => {
                // Value::String("") is the lowest-sorted value
                let range_start = Fact::new(e.clone(), a.clone(), Value::String("".into()));
                self.eav
                    .range(range_start)
                    .take_while(|f| f.entity == e && f.attribute == *a)
                    .collect()
            }
            // FIXME: Implement other optimized index use cases? (multiple unknowns? refs?)
            // Fallthrough case: just scan the EAV index. Correct but slow.
            _ => {
                self.eav
                    .iter()
                    .filter(|f| unify(&binding, &clause, &f).is_ok())
                    .collect()
            }
        }
    }
}

fn unify(env: &Binding, clause: &Clause, fact: &Fact) -> Result<Binding, ()> {
    let mut new_info = HashMap::new();

    match clause.entity {
        Term::Bound(ref e) => {
            if *e != fact.entity {
                return Err(());
            }
        }
        Term::Unbound(ref var) => {
            match env.get(var) {
                Some(e) => {
                    if *e != Value::Entity(fact.entity) {
                        return Err(());
                    }
                }
                _ => {
                    new_info.insert((*var).clone(), Value::Entity(fact.entity));
                }
            }
        }
    }

    match clause.attribute {
        Term::Bound(ref a) => {
            if *a != fact.attribute {
                return Err(());
            }
        }
        Term::Unbound(ref var) => {
            match env.get(var) {
                Some(e) => {
                    if *e != Value::String(fact.attribute.clone()) {
                        return Err(());
                    }
                }
                _ => {
                    new_info.insert((*var).clone(), Value::String(fact.attribute.clone()));
                }
            }
        }
    }

    match clause.value {
        Term::Bound(ref v) => {
            if *v != fact.value {
                return Err(());
            }
        }
        Term::Unbound(ref var) => {
            match env.get(var) {
                Some(e) => {
                    if *e != fact.value {
                        return Err(());
                    }
                }
                _ => {
                    new_info.insert((*var).clone(), fact.value.clone());
                }
            }
        }
    }

    let mut env = env.clone();
    env.extend(new_info);

    Ok(env)
}

#[cfg(test)]
mod test {
    use std::iter;

    use super::*;

    #[test]
    fn test_parse_query() {
        assert_eq!(parse_query("find ?a where (?a name \"Bob\")").unwrap(),
                   Query {
                       find: vec![Var::new("a")],
                       clauses: vec![Clause::new(Term::Unbound("a".into()),
                                                 Term::Bound("name".into()),
                                                 Term::Bound(Value::String("Bob".into())))],
                   })
    }

    #[test]
    fn test_parse_tx() {
        assert_eq!(parse_tx("add (0 name \"Bob\")").unwrap(),
                   Tx {
                       items: vec![TxItem::Addition(Fact::new(Entity(0),
                                                              "name",
                                                              Value::String("Bob".into())))],
                   });
        parse_tx("{name \"Bob\" batch \"S1'17\"}").unwrap();
    }

    lazy_static! {
        static ref DB: InMemoryLog = {
            let mut db = InMemoryLog::new();
            let facts = vec![Fact::new(Entity(0), "name", "Bob"),
                             Fact::new(Entity(1), "name", "John"),
                             Fact::new(Entity(2), "Hello", "World"),
                             Fact::new(Entity(1), "parent", Entity(0))];

            for fact in facts {
                db.add(fact);
            }

            db
        };
    }

    #[test]
    fn test_insertion() {
        let fact = Fact::new(Entity(0), "name", "Bob");
        let mut db = InMemoryLog::new();
        db.add(fact);
        let inserted = db.into_iter().take(1).nth(0).unwrap();
        assert!(inserted.entity == Entity(0));
        assert!(inserted.attribute == "name");
        assert!(inserted.value == "Bob".into());
    }

    #[test]
    fn test_facts_matching() {
        assert_eq!(DB.facts_matching(&Clause::new(Term::Unbound("e".into()),
                                                  Term::Bound("name".into()),
                                                  Term::Bound(Value::String("Bob".into()))),
                                     &Binding::default()),
                   vec![&Fact::new(Entity(0), "name", Value::String("Bob".into()))])
    }

    #[test]
    fn test_query_unknown_entity() {
        // find ?a where (?a name "Bob")
        helper(&*DB,
               parse_query("find ?a where (?a name \"Bob\")").unwrap(),
               QueryResult(vec![Var::new("a")],
                           vec![iter::once((Var::new("a"), Value::Entity(Entity(0)))).collect()]));
    }

    #[test]
    fn test_query_unknown_value() {
        // find ?a where (0 name ?a)
        helper(&*DB,
               parse_query("find ?a where (0 name ?a)").unwrap(),
               QueryResult(vec![Var::new("a")],
                           vec![iter::once((Var::new("a"), Value::String("Bob".into())))
                                    .collect()]));

    }
    #[test]
    fn test_query_unknown_attribute() {
        // find ?a where (1 ?a "John")
        helper(&*DB,
               parse_query("find ?a where (1 ?a \"John\")").unwrap(),
               QueryResult(vec![Var::new("a")],
                           vec![iter::once((Var::new("a"), Value::String("name".into())))
                                    .collect()]));
    }

    #[test]
    fn test_query_multiple_results() {
        // find ?a ?b where (?a name ?b)
        helper(&*DB,
               parse_query("find ?a ?b where (?a name ?b)").unwrap(),
               QueryResult(vec![Var::new("a"), Var::new("b")],
                           vec![vec![(Var::new("a"), Value::Entity(Entity(0))),
                                     (Var::new("b"), Value::String("Bob".into()))]
                                    .into_iter()
                                    .collect(),
                                vec![(Var::new("a"), Value::Entity(Entity(1))),
                                     (Var::new("b"), Value::String("John".into()))]
                                    .into_iter()
                                    .collect()]));
    }

    #[test]
    fn test_query_explicit_join() {
        // find ?b where (?a name Bob) (?b parent ?a)
        helper(&*DB,
               parse_query("find ?b where (?a name \"Bob\") (?b parent ?a)").unwrap(),
               QueryResult(vec![Var::new("b")],
                           vec![iter::once((Var::new("b"), Value::Entity(Entity(1)))).collect()]));
    }

    #[test]
    fn test_query_implicit_join() {
        // find ?c where (?a name Bob) (?b name ?c) (?b parent ?a)
        helper(&*DB,
               parse_query("find ?c where (?a name \"Bob\") (?b name ?c) (?b parent ?a)").unwrap(),
               QueryResult(vec![Var::new("c")],
                           vec![iter::once((Var::new("c"), Value::String("John".into())))
                                    .collect()]));
    }

    fn helper<D: Database>(db: &D, query: Query, expected: QueryResult) {
        let result = db.query(query);
        assert_eq!(expected, result);
    }
}
