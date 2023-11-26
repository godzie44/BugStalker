use std::collections::HashMap;

/// Index data structure. All (path, value) pair unfolds into this structure.
/// For example fold a ("/one/two/three", 101) pair, head is "three", and tail is `["one", "two"]`
/// first add tail into `tail` vector, then add a head ("three") and
/// a pair (an index of tail, id of a head - a nonce) into `heads` hashmap (now heads referenced to one or more tails),
/// and at last, add (head nonce, tail index) and a value into `data`
/// hashmap (now combination of head id and tail index referenced to a data).
#[derive(Clone, Debug)]
struct PathIndexInner<T> {
    next_nonce: u64,
    heads: HashMap<String, (Vec<usize>, u64)>,
    tails: Vec<Vec<String>>,
    data: HashMap<(u64, usize), T>,
}

/// A data structure for storing key-value values, where the key is a path. For example
/// key may be a function path (e.g. "namespace_1::namespace_2::my_fn") or a file path.
///
/// This index is optimized for queries along the last parts of the path. Optimization is based on
/// the expectation that the last part of the path (head) has a high cardinality.
///
/// For example, if index contain function paths like "namespace_1::namespace_2::my_fn" than acceptable
/// requests are:
/// - "my_fn"
/// - "namespace_2::my_fn"
/// - "namespace_1::namespace_2::my_fn"
/// Other requests are not accepted.
#[derive(Clone, Debug)]
pub struct PathSearchIndex<T> {
    delimiter: String,
    index: PathIndexInner<T>,
}

impl<T> PathSearchIndex<T> {
    /// Create a new path index.
    ///
    /// # Arguments
    ///
    /// * `delimiter`: path delimiter
    pub fn new(delimiter: impl Into<String>) -> Self {
        Self {
            delimiter: delimiter.into(),
            index: PathIndexInner {
                next_nonce: 0,
                heads: HashMap::default(),
                tails: Vec::default(),
                data: HashMap::default(),
            },
        }
    }

    /// Insert a new index value.
    ///
    /// # Arguments
    ///
    /// * `path`: an iterator over path parts
    /// * `value`: a value associated with path
    #[allow(unused)]
    pub fn insert(&mut self, path: impl IntoIterator<Item = impl ToString>, value: T) {
        let path: Vec<_> = path.into_iter().map(|p| p.to_string()).collect();
        let Some(head) = path.last() else {
            return;
        };

        self.insert_w_head(path[..path.len() - 1].iter(), head.to_string(), value)
    }

    /// Insert a new index value.
    ///
    /// # Arguments
    ///
    /// * `path`: an iterator over path parts exclude last part
    /// * `head`: a last path part
    /// * `value`: a value associated with path
    pub fn insert_w_head(
        &mut self,
        path: impl IntoIterator<Item = impl ToString>,
        head: impl ToString,
        value: T,
    ) {
        let index = &mut self.index;

        let path: Vec<_> = path.into_iter().map(|p| p.to_string()).collect();
        let head = head.to_string();

        let tail = path;
        index.tails.push(tail);
        let tail_idx = index.tails.len() - 1;

        let head_entry = index.heads.entry(head).or_insert_with(|| {
            let new_rec = (vec![], index.next_nonce);
            index.next_nonce += 1;
            new_rec
        });
        head_entry.0.push(tail_idx);
        index.data.insert((head_entry.1, tail_idx), value);
    }

    /// Return all values which correspond to sub-path.
    ///
    /// # Arguments
    ///
    /// * `needle`: one or more parts in the end of target path
    pub fn get(&self, needle: &str) -> Vec<&T> {
        let mut split: Vec<_> = needle
            .split(&self.delimiter)
            .map(|part| part.to_string())
            .collect();

        let Some(expected_head) = split.pop() else {
            return vec![];
        };
        let expected_tail = split;

        let Some((tail_indexes, head_nonce)) = self.index.heads.get(&expected_head) else {
            return vec![];
        };

        let tail_indexes = tail_indexes.iter().filter(|&&idx| {
            let tail = &self.index.tails[idx];
            tail.ends_with(&expected_tail)
        });

        tail_indexes
            .filter_map(|tail_idx| self.index.data.get(&(*head_nonce, *tail_idx)))
            .collect()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn fill_index(index: &mut PathSearchIndex<i32>) {
        index.insert(["ns1", "ns2", "fn1"], 10);
        index.insert(["ns3", "ns2", "fn1"], 11);
        index.insert(["ns1", "fn2"], 2);
        index.insert(["ns1", "ns2", "fn3"], 3);
        index.insert(["fn4"], 4);
        index.insert(["fn5"], 5);
        index.insert(["ns3", "ns2", "fn3"], 6);
        index.insert(["ns3", "ns2", "fn6"], 7);
        index.insert(["ns3", "ns2", "fn7"], 8);
        index.insert(["ns3", "ns2", "fn8"], 9);
    }

    fn fill_index_with_head(index: &mut PathSearchIndex<i32>) {
        index.insert_w_head(["ns1", "ns2"], "fn1", 10);
        index.insert_w_head(["ns3", "ns2"], "fn1", 11);
        index.insert_w_head(["ns1"], "fn2", 2);
        index.insert_w_head(["ns1", "ns2"], "fn3", 3);
        index.insert_w_head(&Vec::<&str>::new(), "fn4", 4);
        index.insert_w_head(&Vec::<&str>::new(), "fn5", 5);
        index.insert_w_head(["ns3", "ns2"], "fn3", 6);
        index.insert_w_head(["ns3", "ns2"], "fn6", 7);
        index.insert_w_head(["ns3", "ns2"], "fn7", 8);
        index.insert_w_head(["ns3", "ns2"], "fn8", 9);
    }

    #[test]
    pub fn test_index() {
        let mut index = PathSearchIndex::new("::");
        fill_index(&mut index);

        assert_eq!(index.get("ns1::ns2"), Vec::<&i32>::new());
        assert_eq!(index.get(""), Vec::<&i32>::new());
        assert_eq!(index.get("fn"), Vec::<&i32>::new());
        assert_eq!(index.get("ns1::ns2::fn1"), vec![&10]);
        assert_eq!(index.get("ns2::fn1"), vec![&10, &11]);
        assert_eq!(index.get("fn1"), vec![&10, &11]);
        assert_eq!(index.get("fn4"), vec![&4]);
        assert_eq!(index.get("fn5"), vec![&5]);
        assert_eq!(index.get("s1::ns2::fn1"), Vec::<&i32>::new());
        assert_eq!(index.get("ns1::ns2::fn3"), vec![&3]);
        assert_eq!(index.get("ns3::ns2::fn3"), vec![&6]);
        assert_eq!(index.get("ns3::ns2::fn6"), vec![&7]);
        assert_eq!(index.get("ns3::ns2::fn8"), vec![&9]);
    }

    #[test]
    pub fn test_index_2() {
        let mut index = PathSearchIndex::new("::");
        fill_index_with_head(&mut index);

        assert_eq!(index.get("ns1::ns2"), Vec::<&i32>::new());
        assert_eq!(index.get(""), Vec::<&i32>::new());
        assert_eq!(index.get("fn"), Vec::<&i32>::new());
        assert_eq!(index.get("ns1::ns2::fn1"), vec![&10]);
        assert_eq!(index.get("ns2::fn1"), vec![&10, &11]);
        assert_eq!(index.get("fn1"), vec![&10, &11]);
        assert_eq!(index.get("fn4"), vec![&4]);
        assert_eq!(index.get("fn5"), vec![&5]);
        assert_eq!(index.get("s1::ns2::fn1"), Vec::<&i32>::new());
        assert_eq!(index.get("ns1::ns2::fn3"), vec![&3]);
        assert_eq!(index.get("ns3::ns2::fn3"), vec![&6]);
        assert_eq!(index.get("ns3::ns2::fn6"), vec![&7]);
        assert_eq!(index.get("ns3::ns2::fn8"), vec![&9]);
    }
}
