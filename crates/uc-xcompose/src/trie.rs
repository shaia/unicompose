//! An arena trie: sequences of keys to values. A node may hold a value and have children
//! at the same time; `RuleSet` never builds such a node, but the engine's table can.

/// A node in a `Trie`. Only valid for the trie that returned it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(u32);

#[derive(Debug, Clone)]
struct Node<K, V> {
    /// Sorted by key.
    children: Vec<(K, NodeId)>,
    value: Option<V>,
}

#[derive(Debug, Clone)]
pub struct Trie<K, V> {
    nodes: Vec<Node<K, V>>,
}

impl<K: Ord + Copy, V> Default for Trie<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Ord + Copy, V> Trie<K, V> {
    pub fn new() -> Self {
        Trie { nodes: vec![Node { children: Vec::new(), value: None }] }
    }

    /// The empty sequence.
    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    pub fn child(&self, node: NodeId, key: K) -> Option<NodeId> {
        let children = &self.node(node).children;
        children.binary_search_by(|(k, _)| k.cmp(&key)).ok().map(|i| children[i].1)
    }

    /// The child for `key`, created if it does not exist.
    pub fn child_or_insert(&mut self, node: NodeId, key: K) -> NodeId {
        let next = NodeId(u32::try_from(self.nodes.len()).expect("fewer than 2^32 nodes"));
        let children = &self.nodes[node.0 as usize].children;
        match children.binary_search_by(|(k, _)| k.cmp(&key)) {
            Ok(i) => children[i].1,
            Err(i) => {
                self.nodes[node.0 as usize].children.insert(i, (key, next));
                self.nodes.push(Node { children: Vec::new(), value: None });
                next
            }
        }
    }

    /// Children in key order.
    pub fn children(&self, node: NodeId) -> impl Iterator<Item = (K, NodeId)> + '_ {
        self.node(node).children.iter().copied()
    }

    pub fn has_children(&self, node: NodeId) -> bool {
        !self.node(node).children.is_empty()
    }

    /// Detaches every child of `node`. Their nodes stay in the arena, unreachable.
    pub fn clear_children(&mut self, node: NodeId) {
        self.nodes[node.0 as usize].children.clear();
    }

    pub fn value(&self, node: NodeId) -> Option<&V> {
        self.node(node).value.as_ref()
    }

    pub fn set_value(&mut self, node: NodeId, value: Option<V>) -> Option<V> {
        std::mem::replace(&mut self.nodes[node.0 as usize].value, value)
    }

    /// The node for `keys`, if every step exists.
    pub fn find(&self, keys: &[K]) -> Option<NodeId> {
        keys.iter().try_fold(self.root(), |node, &key| self.child(node, key))
    }

    /// Every reachable value with its key sequence, in key order.
    pub fn entries(&self) -> Vec<(Vec<K>, &V)> {
        let mut out = Vec::new();
        let mut path = Vec::new();
        self.collect(self.root(), &mut path, &mut out);
        out
    }

    fn collect<'a>(&'a self, node: NodeId, path: &mut Vec<K>, out: &mut Vec<(Vec<K>, &'a V)>) {
        if let Some(value) = self.value(node) {
            out.push((path.clone(), value));
        }
        for (key, child) in self.children(node) {
            path.push(key);
            self.collect(child, path, out);
            path.pop();
        }
    }

    fn node(&self, node: NodeId) -> &Node<K, V> {
        &self.nodes[node.0 as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_finds_and_lists_in_key_order() {
        let mut trie = Trie::new();
        for (keys, value) in [(&[2, 1][..], "b"), (&[1][..], "a"), (&[2, 3][..], "c")] {
            let node = keys.iter().fold(trie.root(), |n, &k| trie.child_or_insert(n, k));
            trie.set_value(node, Some(value));
        }
        assert_eq!(trie.find(&[2, 3]).and_then(|n| trie.value(n)), Some(&"c"));
        assert_eq!(trie.find(&[2]).and_then(|n| trie.value(n)), None);
        assert!(trie.find(&[3]).is_none());
        let entries: Vec<_> = trie.entries().into_iter().map(|(k, v)| (k, *v)).collect();
        assert_eq!(entries, [(vec![1], "a"), (vec![2, 1], "b"), (vec![2, 3], "c")]);
    }
}
