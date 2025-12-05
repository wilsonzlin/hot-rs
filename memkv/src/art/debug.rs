//! Debug utilities for ART troubleshooting.

use super::{Node, AdaptiveRadixTree};

impl<V: std::fmt::Debug + Clone> AdaptiveRadixTree<V> {
    /// Print the tree structure for debugging.
    pub fn debug_print(&self) {
        println!("=== ART Debug ===");
        println!("Size: {}", self.size);
        if let Some(ref root) = self.root {
            Self::debug_node(root, 0, "");
        } else {
            println!("(empty)");
        }
        println!("=================");
    }

    fn debug_node(node: &Node<V>, depth: usize, path: &str) {
        let indent = "  ".repeat(depth);
        match node {
            Node::Leaf { key, value } => {
                println!("{}Leaf: {:?} -> {:?}", indent, 
                    String::from_utf8_lossy(key), value);
            }
            Node::Node4 { prefix, keys, num_children, children, leaf_value, .. } => {
                println!("{}Node4 (prefix={:?}, children={})", 
                    indent, String::from_utf8_lossy(prefix), num_children);
                if let Some((k, _)) = leaf_value {
                    println!("{}  [leaf_value at {:?}]", indent, String::from_utf8_lossy(k));
                }
                for i in 0..*num_children as usize {
                    let key_byte = keys[i];
                    let new_path = format!("{}{}", path, key_byte as char);
                    println!("{}  [{}] ->", indent, key_byte as char);
                    Self::debug_node(&children[i], depth + 2, &new_path);
                }
            }
            Node::Node16 { prefix, keys, num_children, children, leaf_value, .. } => {
                println!("{}Node16 (prefix={:?}, children={})", 
                    indent, String::from_utf8_lossy(prefix), num_children);
                if let Some((k, _)) = leaf_value {
                    println!("{}  [leaf_value at {:?}]", indent, String::from_utf8_lossy(k));
                }
                for i in 0..*num_children as usize {
                    let key_byte = keys[i];
                    println!("{}  [{}] ->", indent, key_byte as char);
                    Self::debug_node(&children[i], depth + 2, path);
                }
            }
            Node::Node48 { prefix, child_index, num_children, children, leaf_value, .. } => {
                println!("{}Node48 (prefix={:?}, children={})", 
                    indent, String::from_utf8_lossy(prefix), num_children);
                if let Some((k, _)) = leaf_value {
                    println!("{}  [leaf_value at {:?}]", indent, String::from_utf8_lossy(k));
                }
                for byte in 0..=255u8 {
                    let idx = child_index[byte as usize];
                    if idx != 255 && (idx as usize) < children.len() {
                        println!("{}  [{}] ->", indent, byte as char);
                        Self::debug_node(&children[idx as usize], depth + 2, path);
                    }
                }
            }
            Node::Node256 { prefix, num_children, children, leaf_value, .. } => {
                println!("{}Node256 (prefix={:?}, children={})", 
                    indent, String::from_utf8_lossy(prefix), num_children);
                if let Some((k, _)) = leaf_value {
                    println!("{}  [leaf_value at {:?}]", indent, String::from_utf8_lossy(k));
                }
                for byte in 0..256 {
                    if let Some(ref child) = children[byte] {
                        println!("{}  [{}] ->", indent, byte as u8 as char);
                        Self::debug_node(child, depth + 2, path);
                    }
                }
            }
        }
    }

    /// Verify tree integrity - returns list of issues found.
    pub fn verify_integrity(&self) -> Vec<String> {
        let mut issues = Vec::new();
        if let Some(ref root) = self.root {
            Self::verify_node(root, &mut issues, Vec::new());
        }
        issues
    }

    fn verify_node(node: &Node<V>, issues: &mut Vec<String>, path: Vec<u8>) {
        match node {
            Node::Leaf { key, .. } => {
                // Leaf should have a valid key
                if key.is_empty() && !path.is_empty() {
                    issues.push(format!("Leaf at path {:?} has empty key", path));
                }
            }
            Node::Node4 { num_children, children, keys, .. } => {
                let n = *num_children as usize;
                if n > 4 {
                    issues.push(format!("Node4 has {} children (max 4)", n));
                }
                if children.len() < n {
                    issues.push(format!("Node4 children.len()={} < num_children={}", 
                        children.len(), n));
                }
                // Check for duplicate keys
                for i in 0..n {
                    for j in (i+1)..n {
                        if keys[i] == keys[j] {
                            issues.push(format!("Node4 has duplicate key {}", keys[i]));
                        }
                    }
                }
                for i in 0..n.min(children.len()) {
                    let mut new_path = path.clone();
                    new_path.push(keys[i]);
                    Self::verify_node(&children[i], issues, new_path);
                }
            }
            Node::Node16 { num_children, children, keys, .. } => {
                let n = *num_children as usize;
                if n > 16 {
                    issues.push(format!("Node16 has {} children (max 16)", n));
                }
                if children.len() < n {
                    issues.push(format!("Node16 children.len()={} < num_children={}", 
                        children.len(), n));
                }
                for i in 0..n.min(children.len()) {
                    let mut new_path = path.clone();
                    new_path.push(keys[i]);
                    Self::verify_node(&children[i], issues, new_path);
                }
            }
            Node::Node48 { num_children, children, child_index, .. } => {
                let n = *num_children as usize;
                if n > 48 {
                    issues.push(format!("Node48 has {} children (max 48)", n));
                }
                let valid_indices: Vec<_> = child_index.iter()
                    .enumerate()
                    .filter(|(_, &idx)| idx != 255)
                    .collect();
                if valid_indices.len() != n {
                    issues.push(format!("Node48 has {} valid indices but num_children={}", 
                        valid_indices.len(), n));
                }
                for (byte, &idx) in &valid_indices {
                    if (idx as usize) >= children.len() {
                        issues.push(format!("Node48 index {} >= children.len() {}", 
                            idx, children.len()));
                    } else {
                        let mut new_path = path.clone();
                        new_path.push(*byte as u8);
                        Self::verify_node(&children[idx as usize], issues, new_path);
                    }
                }
            }
            Node::Node256 { num_children, children, .. } => {
                let actual_count = children.iter().filter(|c| c.is_some()).count();
                if actual_count != *num_children as usize {
                    issues.push(format!("Node256 has {} Some children but num_children={}", 
                        actual_count, num_children));
                }
                for (byte, child_opt) in children.iter().enumerate() {
                    if let Some(child) = child_opt {
                        let mut new_path = path.clone();
                        new_path.push(byte as u8);
                        Self::verify_node(child, issues, new_path);
                    }
                }
            }
        }
    }
}
