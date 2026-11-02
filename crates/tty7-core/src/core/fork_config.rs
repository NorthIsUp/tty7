//! The fork's settings, kept out of upstream's `Config` so a rebase touches
//! one field there: `Config::fork`, flattened, so they sit at the top level
//! of `config.json` exactly as they did when they were `Config`'s own.

// Every feature's fields use these; the empty section itself does not.
#[allow(unused_imports)]
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[allow(unused_imports)]
use super::config::de_lenient;

/// Every default is in [`Default`] below: the struct-level `serde(default)`
/// fills a missing key from it, so no field names its own.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ForkConfig {
}

impl Default for ForkConfig {
    fn default() -> Self {
        Self {
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::config::Config;

    /// A fork key that upstream's `Config` also had would be written twice
    /// and read by only one of them.
    #[test]
    fn fork_keys_and_upstream_keys_do_not_overlap() {
        /// The top-level keys of a JSON object as written, duplicates kept
        /// (a `serde_json::Value` would fold them into one).
        struct Keys(Vec<String>);
        impl<'de> serde::Deserialize<'de> for Keys {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct V;
                impl<'de> serde::de::Visitor<'de> for V {
                    type Value = Keys;
                    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                        f.write_str("an object")
                    }
                    fn visit_map<A: serde::de::MapAccess<'de>>(
                        self,
                        mut map: A,
                    ) -> Result<Keys, A::Error> {
                        let mut keys = Vec::new();
                        while let Some((k, serde::de::IgnoredAny)) = map.next_entry()? {
                            keys.push(k);
                        }
                        Ok(Keys(keys))
                    }
                }
                d.deserialize_map(V)
            }
        }
        let keys = |text: String| serde_json::from_str::<Keys>(&text).unwrap().0;
        let fork = keys(serde_json::to_string(&super::ForkConfig::default()).unwrap());
        let all = keys(serde_json::to_string(&Config::default()).unwrap());
        let unique: std::collections::BTreeSet<_> = all.iter().collect();
        assert_eq!(unique.len(), all.len(), "a key written twice");
        assert!(
            fork.iter().all(|k| unique.contains(k)),
            "every fork key reaches the file"
        );
    }
}
