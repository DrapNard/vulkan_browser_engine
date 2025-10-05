pub mod resolver;

use base64::Engine;
pub use resolver::*;

use async_recursion::async_recursion;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::log;

pub struct ModuleSystem {
    module_cache: Arc<RwLock<HashMap<String, Arc<Module>>>>,
    resolver: ModuleResolver,
    loader: ModuleLoader,
}

#[derive(Clone)]
pub struct Module {
    pub id: String,
    pub source: String,
    pub exports: HashMap<String, ModuleExport>,
    pub dependencies: Vec<String>,
    pub loaded: bool,
    pub loading: bool,
}

#[derive(Clone)]
pub enum ModuleExport {
    Function(String),
    Object(HashMap<String, ModuleExport>),
    Value(serde_json::Value),
}

pub struct ModuleLoader {
    #[allow(dead_code)]
    base_url: String,
    #[allow(dead_code)]
    allowed_schemes: Vec<String>,
}

impl ModuleSystem {
    pub fn new() -> Self {
        Self {
            module_cache: Arc::new(RwLock::new(HashMap::new())),
            resolver: ModuleResolver::new(),
            loader: ModuleLoader::new(),
        }
    }
    #[async_recursion(?Send)]
    pub async fn import_module(
        &self,
        specifier: &str,
        referrer: Option<String>,
    ) -> Result<Arc<Module>, ModuleError> {
        let resolved_url = self.resolver.resolve(specifier, referrer.as_deref())?;

        {
            let cache = self.module_cache.read().await;
            if let Some(module) = cache.get(&resolved_url) {
                if module.loaded {
                    return Ok(module.clone());
                }
            }
        }

        let source = self.loader.load(&resolved_url).await?;
        let dependencies = self.extract_dependencies(&source)?;

        let module = Arc::new(Module {
            id: resolved_url.clone(),
            source,
            exports: HashMap::new(),
            dependencies,
            loaded: false,
            loading: true,
        });

        {
            let mut cache = self.module_cache.write().await;
            cache.insert(resolved_url.clone(), module.clone());
        }

        for dep in &module.dependencies {
            self.import_module(dep, Some(resolved_url.clone())).await?;
        }

        self.execute_module(&module).await?;

        {
            let mut cache = self.module_cache.write().await;
            if let Some(cached_module) = cache.get_mut(&resolved_url) {
                let module_mut = Arc::make_mut(cached_module);
                module_mut.loaded = true;
                module_mut.loading = false;
            }
        }

        Ok(module)
    }

    async fn execute_module(&self, module: &Module) -> Result<(), ModuleError> {
        log::info!("Executing module: {}", module.id);
        Ok(())
    }

    fn extract_dependencies(&self, source: &str) -> Result<Vec<String>, ModuleError> {
        let mut dependencies = Vec::new();

        for line in source.lines() {
            let trimmed = line.trim();
            if let Some(import_spec) = self.parse_import_statement(trimmed) {
                dependencies.push(import_spec);
            }
        }

        Ok(dependencies)
    }

    fn parse_import_statement(&self, line: &str) -> Option<String> {
        if line.starts_with("import ") {
            if let Some(from_pos) = line.find(" from ") {
                let spec_part = &line[from_pos + 6..].trim();
                if spec_part.starts_with('"') && spec_part.ends_with('"') {
                    return Some(spec_part[1..spec_part.len() - 1].to_string());
                } else if spec_part.starts_with('\'') && spec_part.ends_with('\'') {
                    return Some(spec_part[1..spec_part.len() - 1].to_string());
                }
            }
        }
        None
    }

    pub async fn register_builtin_module(
        &self,
        name: &str,
        exports: HashMap<String, ModuleExport>,
    ) {
        let module = Arc::new(Module {
            id: name.to_string(),
            source: String::new(),
            exports,
            dependencies: Vec::new(),
            loaded: true,
            loading: false,
        });

        let mut cache = self.module_cache.write().await;
        cache.insert(name.to_string(), module);
    }

    pub async fn get_module_graph(&self) -> ModuleGraph {
        let cache = self.module_cache.read().await;
        let mut graph = ModuleGraph::new();

        for (id, module) in cache.iter() {
            graph.add_module(id.clone());
            for dep in &module.dependencies {
                graph.add_dependency(id.clone(), dep.clone());
            }
        }

        graph
    }
}

impl ModuleLoader {
    fn new() -> Self {
        Self {
            base_url: "file://".to_string(),
            allowed_schemes: vec!["file".to_string(), "https".to_string(), "data".to_string()],
        }
    }

    async fn load(&self, url: &str) -> Result<String, ModuleError> {
        if url.starts_with("https://") {
            self.load_remote(url).await
        } else if url.starts_with("file://") {
            self.load_file(url).await
        } else if url.starts_with("data:") {
            self.load_data_url(url).await
        } else {
            Err(ModuleError::UnsupportedScheme(url.to_string()))
        }
    }

    async fn load_remote(&self, url: &str) -> Result<String, ModuleError> {
        let response = reqwest::get(url)
            .await
            .map_err(|e| ModuleError::LoadError(e.to_string()))?;

        response
            .text()
            .await
            .map_err(|e| ModuleError::LoadError(e.to_string()))
    }

    async fn load_file(&self, url: &str) -> Result<String, ModuleError> {
        let path = url.strip_prefix("file://").unwrap_or(url);
        tokio::fs::read_to_string(path)
            .await
            .map_err(|e| ModuleError::LoadError(e.to_string()))
    }

    async fn load_data_url(&self, url: &str) -> Result<String, ModuleError> {
        if let Some(comma_pos) = url.find(',') {
            let data_part = &url[comma_pos + 1..];
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(data_part)
                .map_err(|e| ModuleError::LoadError(e.to_string()))?;
            String::from_utf8(decoded).map_err(|e| ModuleError::LoadError(e.to_string()))
        } else {
            Err(ModuleError::InvalidDataUrl)
        }
    }
}

pub struct ModuleGraph {
    nodes: HashMap<String, ModuleNode>,
    edges: Vec<(String, String)>,
}

struct ModuleNode {
    #[allow(dead_code)]
    id: String,
    dependencies: Vec<String>,
    #[allow(dead_code)]
    dependents: Vec<String>,
}

impl ModuleGraph {
    fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            edges: Vec::new(),
        }
    }

    fn add_module(&mut self, id: String) {
        self.nodes.entry(id.clone()).or_insert_with(|| ModuleNode {
            id,
            dependencies: Vec::new(),
            dependents: Vec::new(),
        });
    }

    fn add_dependency(&mut self, from: String, to: String) {
        self.add_module(from.clone());
        self.add_module(to.clone());

        if let Some(node) = self.nodes.get_mut(&from) {
            if !node.dependencies.contains(&to) {
                node.dependencies.push(to.clone());
            }
        }

        if let Some(node) = self.nodes.get_mut(&to) {
            if !node.dependents.contains(&from) {
                node.dependents.push(from.clone());
            }
        }

        self.edges.push((from, to));
    }

    pub fn detect_cycles(&self) -> Vec<Vec<String>> {
        let mut cycles = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut rec_stack = std::collections::HashSet::new();

        for node_id in self.nodes.keys() {
            if !visited.contains(node_id) {
                self.dfs_cycle_detection(
                    node_id,
                    &mut visited,
                    &mut rec_stack,
                    &mut cycles,
                    &mut Vec::new(),
                );
            }
        }

        cycles
    }

    fn dfs_cycle_detection(
        &self,
        node: &str,
        visited: &mut std::collections::HashSet<String>,
        rec_stack: &mut std::collections::HashSet<String>,
        cycles: &mut Vec<Vec<String>>,
        path: &mut Vec<String>,
    ) {
        visited.insert(node.to_string());
        rec_stack.insert(node.to_string());
        path.push(node.to_string());

        if let Some(node_data) = self.nodes.get(node) {
            for dep in &node_data.dependencies {
                if !visited.contains(dep) {
                    self.dfs_cycle_detection(dep, visited, rec_stack, cycles, path);
                } else if rec_stack.contains(dep) {
                    if let Some(cycle_start) = path.iter().position(|x| x == dep) {
                        cycles.push(path[cycle_start..].to_vec());
                    }
                }
            }
        }

        path.pop();
        rec_stack.remove(node);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ModuleError {
    #[error("Resolution error: {0}")]
    ResolutionError(String),
    #[error("Load error: {0}")]
    LoadError(String),
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("Execution error: {0}")]
    ExecutionError(String),
    #[error("Circular dependency detected")]
    CircularDependency,
    #[error("Module not found: {0}")]
    ModuleNotFound(String),
    #[error("Unsupported scheme: {0}")]
    UnsupportedScheme(String),
    #[error("Invalid data URL")]
    InvalidDataUrl,
}

impl From<resolver::ResolveError> for ModuleError {
    fn from(err: resolver::ResolveError) -> Self {
        ModuleError::ResolutionError(err.to_string())
    }
}

impl Default for ModuleSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for ModuleLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for ModuleGraph {
    fn default() -> Self {
        Self::new()
    }
}
