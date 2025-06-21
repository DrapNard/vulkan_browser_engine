use std::path::{Path, PathBuf};
use url::Url;

pub struct ModuleResolver {
    base_url: Option<Url>,
    import_map: ImportMap,
}

pub struct ImportMap {
    imports: std::collections::HashMap<String, String>,
    scopes: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
}

impl ModuleResolver {
    pub fn new() -> Self {
        Self {
            base_url: None,
            import_map: ImportMap::new(),
        }
    }

    pub fn with_base_url(base_url: Url) -> Self {
        Self {
            base_url: Some(base_url),
            import_map: ImportMap::new(),
        }
    }

    pub fn resolve(&self, specifier: &str, referrer: Option<&str>) -> Result<String, ResolveError> {
        if let Some(mapped) = self.import_map.resolve(specifier, referrer) {
            return Ok(mapped);
        }

        if specifier.starts_with("http://") || specifier.starts_with("https://") {
            return Ok(specifier.to_string());
        }

        if specifier.starts_with("file://") {
            return Ok(specifier.to_string());
        }

        if specifier.starts_with("./") || specifier.starts_with("../") {
            return self.resolve_relative(specifier, referrer);
        }

        if specifier.starts_with("/") {
            return self.resolve_absolute(specifier);
        }

        self.resolve_bare(specifier)
    }

    fn resolve_relative(&self, specifier: &str, referrer: Option<&str>) -> Result<String, ResolveError> {
        let base = if let Some(ref_url) = referrer {
            if let Ok(url) = Url::parse(ref_url) {
                url
            } else if let Some(base) = &self.base_url {
                base.clone()
            } else {
                return Err(ResolveError::NoBase);
            }
        } else if let Some(base) = &self.base_url {
            base.clone()
        } else {
            return Err(ResolveError::NoBase);
        };

        let resolved = base.join(specifier)
            .map_err(|e| ResolveError::InvalidUrl(e.to_string()))?;
        
        Ok(resolved.to_string())
    }

    fn resolve_absolute(&self, specifier: &str) -> Result<String, ResolveError> {
        if let Some(base) = &self.base_url {
            let resolved = base.join(specifier)
                .map_err(|e| ResolveError::InvalidUrl(e.to_string()))?;
            Ok(resolved.to_string())
        } else {
            Ok(format!("file://{}", specifier))
        }
    }

    fn resolve_bare(&self, specifier: &str) -> Result<String, ResolveError> {
        if self.is_builtin_module(specifier) {
            return Ok(format!("builtin:{}", specifier));
        }

        if let Some(resolved) = self.resolve_node_modules(specifier) {
            return Ok(resolved);
        }

        Err(ResolveError::ModuleNotFound(specifier.to_string()))
    }

    fn is_builtin_module(&self, specifier: &str) -> bool {
        matches!(specifier, "crypto" | "path" | "fs" | "url" | "stream" | "events" | "util")
    }

    fn resolve_node_modules(&self, specifier: &str) -> Option<String> {
        let mut current_dir = std::env::current_dir().ok()?;
        
        loop {
            let node_modules = current_dir.join("node_modules");
            let module_path = node_modules.join(specifier);
            
            if module_path.exists() {
                let package_json = module_path.join("package.json");
                if package_json.exists() {
                    if let Ok(main) = self.read_package_main(&package_json) {
                        let main_path = module_path.join(main);
                        if main_path.exists() {
                            return Some(format!("file://{}", main_path.display()));
                        }
                    }
                }
                
                let index_js = module_path.join("index.js");
                if index_js.exists() {
                    return Some(format!("file://{}", index_js.display()));
                }
            }
            
            if !current_dir.pop() {
                break;
            }
        }
        
        None
    }

    fn read_package_main(&self, package_json: &Path) -> Result<String, std::io::Error> {
        let content = std::fs::read_to_string(package_json)?;
        let package: serde_json::Value = serde_json::from_str(&content)?;
        
        if let Some(main) = package.get("main").and_then(|v| v.as_str()) {
            Ok(main.to_string())
        } else {
            Ok("index.js".to_string())
        }
    }

    pub fn set_import_map(&mut self, import_map: ImportMap) {
        self.import_map = import_map;
    }
}

impl ImportMap {
    pub fn new() -> Self {
        Self {
            imports: std::collections::HashMap::new(),
            scopes: std::collections::HashMap::new(),
        }
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        let mut import_map = ImportMap::new();

        if let Some(imports) = value.get("imports") {
            if let Some(imports_obj) = imports.as_object() {
                for (key, val) in imports_obj {
                    if let Some(val_str) = val.as_str() {
                        import_map.imports.insert(key.clone(), val_str.to_string());
                    }
                }
            }
        }

        if let Some(scopes) = value.get("scopes") {
            if let Some(scopes_obj) = scopes.as_object() {
                for (scope_key, scope_val) in scopes_obj {
                    if let Some(scope_imports) = scope_val.as_object() {
                        let mut scope_map = std::collections::HashMap::new();
                        for (key, val) in scope_imports {
                            if let Some(val_str) = val.as_str() {
                                scope_map.insert(key.clone(), val_str.to_string());
                            }
                        }
                        import_map.scopes.insert(scope_key.clone(), scope_map);
                    }
                }
            }
        }

        Ok(import_map)
    }

    pub fn resolve(&self, specifier: &str, referrer: Option<&str>) -> Option<String> {
        if let Some(referrer_url) = referrer {
            for (scope_prefix, scope_imports) in &self.scopes {
                if referrer_url.starts_with(scope_prefix) {
                    if let Some(mapped) = scope_imports.get(specifier) {
                        return Some(mapped.clone());
                    }
                }
            }
        }

        self.imports.get(specifier).cloned()
    }

    pub fn add_import(&mut self, specifier: String, url: String) {
        self.imports.insert(specifier, url);
    }

    pub fn add_scoped_import(&mut self, scope: String, specifier: String, url: String) {
        self.scopes
            .entry(scope)
            .or_insert_with(std::collections::HashMap::new)
            .insert(specifier, url);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("Module not found: {0}")]
    ModuleNotFound(String),
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
    #[error("No base URL provided")]
    NoBase,
    #[error("Resolution failed: {0}")]
    ResolutionFailed(String),
}

impl Default for ModuleResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for ImportMap {
    fn default() -> Self {
        Self::new()
    }
}
