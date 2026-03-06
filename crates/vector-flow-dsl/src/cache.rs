use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use dashmap::DashMap;

use crate::ast::DslType;
use crate::codegen::DslCompiler;
use crate::error::DslError;

/// A cached compilation result — either a compiled function pointer or a cached error.
enum CacheEntry {
    Ok(*const u8),
    Err(DslError),
}

// SAFETY: The function pointer points to JIT-compiled code that is valid
// for the lifetime of the JITModule. The DslCompiler (and thus JITModule)
// must outlive any use of these pointers.
unsafe impl Send for CacheEntry {}
unsafe impl Sync for CacheEntry {}

/// Thread-safe function cache keyed by source-code hash.
/// Caches both successful compilations and errors so that invalid source
/// is not re-parsed every frame.
pub struct DslFunctionCache {
    entries: DashMap<u64, CacheEntry>,
}

impl DslFunctionCache {
    pub fn new() -> Self {
        Self {
            entries: DashMap::new(),
        }
    }

    /// Get a compiled function for the given source, compiling on cache miss.
    pub fn get_or_compile_expr(
        &self,
        source: &str,
        compiler: &mut DslCompiler,
    ) -> Result<*const u8, DslError> {
        let hash = hash_source(source);
        self.get_or_insert(hash, || compiler.compile_expression(source))
    }

    /// Get a compiled program function for the given source, compiling on cache miss.
    pub fn get_or_compile_program(
        &self,
        source: &str,
        compiler: &mut DslCompiler,
    ) -> Result<*const u8, DslError> {
        let hash = hash_source(source);
        self.get_or_insert(hash, || compiler.compile_program(source))
    }

    /// Get a compiled node script function, compiling on cache miss.
    /// The cache key incorporates the source AND the port definitions.
    pub fn get_or_compile_node_script(
        &self,
        source: &str,
        inputs: &[(String, DslType)],
        outputs: &[(String, DslType)],
        compiler: &mut DslCompiler,
    ) -> Result<*const u8, DslError> {
        let hash = hash_node_script(source, inputs, outputs);
        self.get_or_insert(hash, || compiler.compile_node_script(source, inputs, outputs))
    }

    fn get_or_insert(
        &self,
        hash: u64,
        compile: impl FnOnce() -> Result<*const u8, DslError>,
    ) -> Result<*const u8, DslError> {
        if let Some(entry) = self.entries.get(&hash) {
            return match entry.value() {
                CacheEntry::Ok(ptr) => Ok(*ptr),
                CacheEntry::Err(e) => Err(e.clone()),
            };
        }

        match compile() {
            Ok(ptr) => {
                self.entries.insert(hash, CacheEntry::Ok(ptr));
                Ok(ptr)
            }
            Err(e) => {
                self.entries.insert(hash, CacheEntry::Err(e.clone()));
                Err(e)
            }
        }
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all cached functions.
    pub fn clear(&self) {
        self.entries.clear();
    }
}

impl Default for DslFunctionCache {
    fn default() -> Self {
        Self::new()
    }
}

fn hash_source(source: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

fn hash_node_script(source: &str, inputs: &[(String, DslType)], outputs: &[(String, DslType)]) -> u64 {
    let mut hasher = DefaultHasher::new();
    "node_script".hash(&mut hasher);
    source.hash(&mut hasher);
    for (name, ty) in inputs {
        name.hash(&mut hasher);
        std::mem::discriminant(ty).hash(&mut hasher);
    }
    for (name, ty) in outputs {
        name.hash(&mut hasher);
        std::mem::discriminant(ty).hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hit_and_miss() {
        let cache = DslFunctionCache::new();
        let mut compiler = DslCompiler::new().unwrap();

        assert!(cache.is_empty());

        let ptr1 = cache.get_or_compile_expr("1.0 + 2.0", &mut compiler).unwrap();
        assert_eq!(cache.len(), 1);

        let ptr2 = cache.get_or_compile_expr("1.0 + 2.0", &mut compiler).unwrap();
        assert_eq!(cache.len(), 1);
        assert_eq!(ptr1, ptr2); // Same pointer from cache

        let _ptr3 = cache.get_or_compile_expr("3.0 * 4.0", &mut compiler).unwrap();
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn cache_clear() {
        let cache = DslFunctionCache::new();
        let mut compiler = DslCompiler::new().unwrap();

        cache.get_or_compile_expr("1.0", &mut compiler).unwrap();
        assert_eq!(cache.len(), 1);

        cache.clear();
        assert!(cache.is_empty());
    }
}
