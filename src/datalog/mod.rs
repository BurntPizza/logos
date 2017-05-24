mod query;

use std::collections::HashMap;

// # Initial pass
// A database is just a log of facts. Facts are (entity, attribute, value) triples.
// Attributes and values are both just strings. There are no transactions or histories.


struct QueryResult;
type NewQueryResult = Result<Vec<HashMap<String, Value>>, String>;

#[derive(Debug)]
pub enum Value {
    String(String),
    Integer(i64),
    Ref(u64)
}

// For now, something in a variable position can be either an unbound variable
// (e.g. ?a) or a string literal.
#[derive(Debug)]
pub enum Var {
    Symbol(String),
    StringLit(String)
}

#[derive(Debug)]
pub enum Term<T> {
    Bound(T),
    Unbound(String)
}

#[derive(Debug)]
pub struct Clause {
    entity: Term<u64>,
    // FIXME: attributes should also be allowed to be Vars
    attribute: Term<String>,
    value: Term<Value>
}

impl Clause {
    pub fn new(e: Term<u64>, a: Term<String>, v: Term<Value>) -> Clause {
        Clause { entity: e, attribute: a, value: v}
    }
}

// A query looks like `find ?var where (?var <attribute> <value>)`
#[derive(Debug)]
pub struct Query {
    find: Var,
    clauses: Vec<Clause>
}

impl Query {
    pub fn new(find: Var, clauses: Vec<Clause>) -> Query {
        Query { find: find, clauses: clauses }
    }
}

trait Database {
    fn add(&mut self, fact: &Fact);
    fn query(&self, query: Query) -> QueryResult;
}

#[derive(Debug, PartialEq, Clone)]
struct Fact {
    entity: u64,
    attribute: String,
    value: String
}

impl Fact {
    pub fn new(e: u64, a: &str, v: &str) -> Fact {
        Fact {entity: e, attribute: a.to_owned(), value: v.to_owned()}
    }
}

#[derive(Debug)]
struct InMemoryLog {
    facts: Vec<Fact>
}

impl InMemoryLog {
    pub fn new() -> InMemoryLog {
        InMemoryLog { facts: Vec::new() }
    }
}

impl IntoIterator for InMemoryLog {
    type Item = Fact;
    type IntoIter = ::std::vec::IntoIter<Fact>;

    fn into_iter(self) -> Self::IntoIter {
        self.facts.into_iter()
    }
}

impl Database for InMemoryLog {
    fn add(&mut self, fact: &Fact) {
        self.facts.push((*fact).clone());
    }

    fn query(&self, query: Query) -> QueryResult {
        QueryResult
    }
}

#[cfg(test)]
mod test {
    use datalog::{Fact, InMemoryLog, Database};
//    use parser;

    #[test]
    fn test_insertion() {
        let fact = Fact::new(0, "name", "Bob");
        let mut db = InMemoryLog::new();
        db.add(&fact);
        let inserted = db.into_iter().take(1).nth(0).unwrap();
        assert!(inserted.entity == 0);
        assert!(inserted.attribute == "name");
        assert!(inserted.value == "Bob");
    }

    #[test]
    fn test_query() {
        let mut db = InMemoryLog::new();
        let facts = vec![
            Fact::new(0, "name", "Bob"),
            Fact::new(1, "name", "John")
        ];

        for fact in facts.iter() {
            db.add(fact);
        }

        // let bob_query = parser::parse_Query("find ?person where (?person name \"Bob\")").unwrap();
        // let john_query = parser::parse_Query("find ?person where (?person name \"John\")").unwrap();

        // let bob_result = db.query(bob_query);
        // let john_result = db.query(john_query);

        //TODO actually test the results
    }
}