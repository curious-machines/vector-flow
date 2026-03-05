use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use dashmap::DashMap;

use crate::codegen::DslCompiler;
use crate::error::DslError;

/// A compiled function entry.
struct CompiledFunction {
    func_ptr: *const u8,
    _source_hash: u64,
}

// SAFETY: The function pointer points to JIT-compiled code that is valid
// for the lifetime of the JITModule. The DslCompiler (and thus JITModule)
// must outlive any use of these pointers.
unsafe impl Send for CompiledFunction {}
unsafe impl Sync for CompiledFunction {}

/// Thread-safe function cache keyed by source-code hash.
pub struct DslFunctionCache {
    entries: DashMap<u64, CompiledFunction>,
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

        if let Some(entry) = self.entries.get(&hash) {
            return Ok(entry.func_ptr);
        }

        let func_ptr = compiler.compile_expression(source)?;
        self.entries.insert(hash, CompiledFunction { func_ptr, _source_hash: hash });
        Ok(func_ptr)
    }

    /// Get a compiled program function for the given source, compiling on cache miss.
    pub fn get_or_compile_program(
        &self,
        source: &str,
        compiler: &mut DslCompiler,
    ) -> Result<*const u8, DslError> {
        let hash = hash_source(source);

        if let Some(entry) = self.entries.get(&hash) {
            return Ok(entry.func_ptr);
        }

        let func_ptr = compiler.compile_program(source)?;
        self.entries.insert(hash, CompiledFunction { func_ptr, _source_hash: hash });
        Ok(func_ptr)
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
