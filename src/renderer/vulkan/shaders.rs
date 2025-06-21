use std::sync::Arc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;
use ash::{Device, vk};
use parking_lot::{RwLock, Mutex};
use dashmap::DashMap;
use smallvec::SmallVec;
use thiserror::Error;
use serde::{Serialize, Deserialize};
use ahash::AHasher;
use std::hash::{Hash, Hasher};

use super::device::VulkanDevice;

#[derive(Error, Debug)]
pub enum ShaderError {
    #[error("Shader compilation failed: {0}")]
    Compilation(String),
    #[error("Shader loading failed: {0}")]
    Loading(String),
    #[error("SPIR-V validation failed: {0}")]
    Validation(String),
    #[error("Pipeline creation failed: {0}")]
    Pipeline(String),
    #[error("Shader not found: {0}")]
    NotFound(String),
    #[error("Hot reload failed: {0}")]
    HotReload(String),
}

pub type Result<T> = std::result::Result<T, ShaderError>;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ShaderStage {
    Vertex,
    Fragment,
    Geometry,
    TessellationControl,
    TessellationEvaluation,
    Compute,
    RayGeneration,
    AnyHit,
    ClosestHit,
    Miss,
    Intersection,
    Callable,
    Task,
    Mesh,
}

impl ShaderStage {
    pub fn to_vk_stage(&self) -> vk::ShaderStageFlags {
        match self {
            ShaderStage::Vertex => vk::ShaderStageFlags::VERTEX,
            ShaderStage::Fragment => vk::ShaderStageFlags::FRAGMENT,
            ShaderStage::Geometry => vk::ShaderStageFlags::GEOMETRY,
            ShaderStage::TessellationControl => vk::ShaderStageFlags::TESSELLATION_CONTROL,
            ShaderStage::TessellationEvaluation => vk::ShaderStageFlags::TESSELLATION_EVALUATION,
            ShaderStage::Compute => vk::ShaderStageFlags::COMPUTE,
            ShaderStage::RayGeneration => vk::ShaderStageFlags::RAYGEN_KHR,
            ShaderStage::AnyHit => vk::ShaderStageFlags::ANY_HIT_KHR,
            ShaderStage::ClosestHit => vk::ShaderStageFlags::CLOSEST_HIT_KHR,
            ShaderStage::Miss => vk::ShaderStageFlags::MISS_KHR,
            ShaderStage::Intersection => vk::ShaderStageFlags::INTERSECTION_KHR,
            ShaderStage::Callable => vk::ShaderStageFlags::CALLABLE_KHR,
            ShaderStage::Task => vk::ShaderStageFlags::TASK_NV,
            ShaderStage::Mesh => vk::ShaderStageFlags::MESH_NV,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShaderSource {
    pub glsl_code: String,
    pub entry_point: String,
    pub stage: ShaderStage,
    pub include_paths: Vec<PathBuf>,
    pub defines: HashMap<String, String>,
    pub optimization_level: OptimizationLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OptimizationLevel {
    None,
    Size,
    Performance,
    Debug,
}

#[derive(Debug, Clone)]
pub struct CompiledShader {
    pub spirv_code: Vec<u32>,
    pub module: vk::ShaderModule,
    pub entry_point: String,
    pub stage: ShaderStage,
    pub hash: u64,
    pub reflection_data: ShaderReflection,
}

#[derive(Debug, Clone, Default)]
pub struct ShaderReflection {
    pub descriptor_sets: Vec<DescriptorSetReflection>,
    pub push_constants: Vec<PushConstantReflection>,
    pub input_variables: Vec<InputVariableReflection>,
    pub output_variables: Vec<OutputVariableReflection>,
    pub local_size: Option<[u32; 3]>,
}

#[derive(Debug, Clone)]
pub struct DescriptorSetReflection {
    pub set: u32,
    pub bindings: Vec<DescriptorBindingReflection>,
}

#[derive(Debug, Clone)]
pub struct DescriptorBindingReflection {
    pub binding: u32,
    pub descriptor_type: vk::DescriptorType,
    pub descriptor_count: u32,
    pub stage_flags: vk::ShaderStageFlags,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct PushConstantReflection {
    pub offset: u32,
    pub size: u32,
    pub stage_flags: vk::ShaderStageFlags,
}

#[derive(Debug, Clone)]
pub struct InputVariableReflection {
    pub location: u32,
    pub name: String,
    pub format: vk::Format,
}

#[derive(Debug, Clone)]
pub struct OutputVariableReflection {
    pub location: u32,
    pub name: String,
    pub format: vk::Format,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShaderCacheEntry {
    source_hash: u64,
    spirv_code: Vec<u32>,
    compilation_time: std::time::SystemTime,
    optimization_level: OptimizationLevel,
}

pub struct ShaderCompiler {
    glslang_validator: Option<PathBuf>,
    spirv_tools: Option<PathBuf>,
    optimization_enabled: bool,
    debug_info_enabled: bool,
}

impl ShaderCompiler {
    pub fn new() -> Self {
        Self {
            glslang_validator: Self::find_glslang_validator(),
            spirv_tools: Self::find_spirv_tools(),
            optimization_enabled: true,
            debug_info_enabled: cfg!(debug_assertions),
        }
    }

    fn find_glslang_validator() -> Option<PathBuf> {
        if let Ok(path) = std::env::var("GLSLANG_VALIDATOR_PATH") {
            return Some(PathBuf::from(path));
        }

        let candidates = [
            "glslangValidator",
            "glslangValidator.exe",
            "/usr/bin/glslangValidator",
            "/usr/local/bin/glslangValidator",
        ];

        for candidate in &candidates {
            if Path::new(candidate).exists() {
                return Some(PathBuf::from(candidate));
            }
        }

        None
    }

    fn find_spirv_tools() -> Option<PathBuf> {
        if let Ok(path) = std::env::var("SPIRV_OPT_PATH") {
            return Some(PathBuf::from(path));
        }

        let candidates = [
            "spirv-opt",
            "spirv-opt.exe",
            "/usr/bin/spirv-opt",
            "/usr/local/bin/spirv-opt",
        ];

        for candidate in &candidates {
            if Path::new(candidate).exists() {
                return Some(PathBuf::from(candidate));
            }
        }

        None
    }

    pub fn compile_glsl_to_spirv(&self, source: &ShaderSource) -> Result<Vec<u32>> {
        let temp_dir = std::env::temp_dir();
        let input_file = temp_dir.join(format!("shader_{}.glsl", fastrand::u64(..)));
        let output_file = temp_dir.join(format!("shader_{}.spv", fastrand::u64(..)));

        let mut full_source = String::new();
        
        full_source.push_str("#version 450 core\n");
        
        for (key, value) in &source.defines {
            full_source.push_str(&format!("#define {} {}\n", key, value));
        }
        
        full_source.push('\n');
        full_source.push_str(&source.glsl_code);

        fs::write(&input_file, &full_source)
            .map_err(|e| ShaderError::Compilation(format!("Failed to write temp file: {}", e)))?;

        let glslang = self.glslang_validator.as_ref()
            .ok_or_else(|| ShaderError::Compilation("glslangValidator not found".to_string()))?;

        let mut cmd = std::process::Command::new(glslang);
        cmd.arg("-V")
           .arg(&input_file)
           .arg("-o")
           .arg(&output_file);

        let stage_arg = match source.stage {
            ShaderStage::Vertex => "-S vert",
            ShaderStage::Fragment => "-S frag",
            ShaderStage::Geometry => "-S geom",
            ShaderStage::TessellationControl => "-S tesc",
            ShaderStage::TessellationEvaluation => "-S tese",
            ShaderStage::Compute => "-S comp",
            _ => return Err(ShaderError::Compilation("Unsupported shader stage".to_string())),
        };
        
        cmd.args(stage_arg.split_whitespace());

        if self.debug_info_enabled {
            cmd.arg("-g");
        }

        for include_path in &source.include_paths {
            cmd.arg("-I").arg(include_path);
        }

        let output = cmd.output()
            .map_err(|e| ShaderError::Compilation(format!("Failed to run glslangValidator: {}", e)))?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            let _ = fs::remove_file(&input_file);
            let _ = fs::remove_file(&output_file);
            return Err(ShaderError::Compilation(format!("GLSL compilation failed: {}", error_msg)));
        }

        let mut spirv_code = fs::read(&output_file)
            .map_err(|e| ShaderError::Compilation(format!("Failed to read SPIR-V output: {}", e)))?;

        let _ = fs::remove_file(&input_file);
        let _ = fs::remove_file(&output_file);

        if self.optimization_enabled && source.optimization_level != OptimizationLevel::None {
            spirv_code = self.optimize_spirv(&spirv_code, source.optimization_level)?;
        }

        if spirv_code.len() % 4 != 0 {
            return Err(ShaderError::Compilation("Invalid SPIR-V code length".to_string()));
        }

        let spirv_u32: Vec<u32> = spirv_code.chunks_exact(4)
            .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect();

        self.validate_spirv(&spirv_u32)?;

        Ok(spirv_u32)
    }

    fn optimize_spirv(&self, spirv_code: &[u8], level: OptimizationLevel) -> Result<Vec<u8>> {
        let spirv_opt = self.spirv_tools.as_ref()
            .ok_or_else(|| ShaderError::Compilation("spirv-opt not found".to_string()))?;

        let temp_dir = std::env::temp_dir();
        let input_file = temp_dir.join(format!("shader_{}.spv", fastrand::u64(..)));
        let output_file = temp_dir.join(format!("shader_opt_{}.spv", fastrand::u64(..)));

        fs::write(&input_file, spirv_code)
            .map_err(|e| ShaderError::Compilation(format!("Failed to write temp SPIR-V file: {}", e)))?;

        let mut cmd = std::process::Command::new(spirv_opt);
        cmd.arg(&input_file)
           .arg("-o")
           .arg(&output_file);

        match level {
            OptimizationLevel::Size => {
                cmd.arg("-Os");
            },
            OptimizationLevel::Performance => {
                cmd.arg("-O");
            },
            OptimizationLevel::Debug => {
                cmd.arg("-g");
            },
            OptimizationLevel::None => {},
        }

        let output = cmd.output()
            .map_err(|e| ShaderError::Compilation(format!("Failed to run spirv-opt: {}", e)))?;

        if !output.status.success() {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            let _ = fs::remove_file(&input_file);
            let _ = fs::remove_file(&output_file);
            return Err(ShaderError::Compilation(format!("SPIR-V optimization failed: {}", error_msg)));
        }

        let optimized_code = fs::read(&output_file)
            .map_err(|e| ShaderError::Compilation(format!("Failed to read optimized SPIR-V: {}", e)))?;

        let _ = fs::remove_file(&input_file);
        let _ = fs::remove_file(&output_file);

        Ok(optimized_code)
    }

    fn validate_spirv(&self, spirv_code: &[u32]) -> Result<()> {
        if spirv_code.len() < 5 {
            return Err(ShaderError::Validation("SPIR-V code too short".to_string()));
        }

        if spirv_code[0] != 0x07230203 {
            return Err(ShaderError::Validation("Invalid SPIR-V magic number".to_string()));
        }

        Ok(())
    }

    fn reflect_spirv(&self, spirv_code: &[u32]) -> Result<ShaderReflection> {
        Ok(ShaderReflection::default())
    }
}

pub struct ShaderManager {
    device: Arc<VulkanDevice>,
    compiler: Arc<Mutex<ShaderCompiler>>,
    shader_cache: Arc<DashMap<u64, CompiledShader>>,
    disk_cache: Arc<RwLock<HashMap<u64, ShaderCacheEntry>>>,
    cache_path: PathBuf,
    hot_reload_enabled: bool,
    file_watchers: Arc<RwLock<HashMap<PathBuf, std::time::SystemTime>>>,
}

impl ShaderManager {
    pub async fn new(device: Arc<VulkanDevice>) -> Result<Self> {
        let cache_path = std::env::temp_dir().join("vulkan_browser_shader_cache");
        if !cache_path.exists() {
            fs::create_dir_all(&cache_path)
                .map_err(|e| ShaderError::Loading(format!("Failed to create cache directory: {}", e)))?;
        }

        let disk_cache = Self::load_disk_cache(&cache_path)?;

        Ok(Self {
            device,
            compiler: Arc::new(Mutex::new(ShaderCompiler::new())),
            shader_cache: Arc::new(DashMap::new()),
            disk_cache: Arc::new(RwLock::new(disk_cache)),
            cache_path,
            hot_reload_enabled: cfg!(debug_assertions),
            file_watchers: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    fn load_disk_cache(cache_path: &Path) -> Result<HashMap<u64, ShaderCacheEntry>> {
        let cache_file = cache_path.join("cache.json");
        if !cache_file.exists() {
            return Ok(HashMap::new());
        }

        let cache_data = fs::read_to_string(&cache_file)
            .map_err(|e| ShaderError::Loading(format!("Failed to read cache file: {}", e)))?;

        serde_json::from_str(&cache_data)
            .map_err(|e| ShaderError::Loading(format!("Failed to parse cache file: {}", e)))
    }

    fn save_disk_cache(&self) -> Result<()> {
        let cache_file = self.cache_path.join("cache.json");
        let cache_data = self.disk_cache.read();
        
        let json_data = serde_json::to_string_pretty(&*cache_data)
            .map_err(|e| ShaderError::Loading(format!("Failed to serialize cache: {}", e)))?;

        fs::write(&cache_file, json_data)
            .map_err(|e| ShaderError::Loading(format!("Failed to write cache file: {}", e)))?;

        Ok(())
    }

    fn compute_source_hash(&self, source: &ShaderSource) -> u64 {
        let mut hasher = AHasher::default();
        source.glsl_code.hash(&mut hasher);
        source.entry_point.hash(&mut hasher);
        source.stage.hash(&mut hasher);
        source.defines.hash(&mut hasher);
        source.optimization_level.hash(&mut hasher);
        hasher.finish()
    }

    pub async fn compile_shader(&self, source: ShaderSource) -> Result<Arc<CompiledShader>> {
        let source_hash = self.compute_source_hash(&source);

        if let Some(cached_shader) = self.shader_cache.get(&source_hash) {
            return Ok(Arc::new(cached_shader.clone()));
        }

        let disk_cache = self.disk_cache.read();
        if let Some(cache_entry) = disk_cache.get(&source_hash) {
            let module = self.create_shader_module(&cache_entry.spirv_code)?;
            let reflection = self.compiler.lock().reflect_spirv(&cache_entry.spirv_code)?;
            
            let compiled_shader = CompiledShader {
                spirv_code: cache_entry.spirv_code.clone(),
                module,
                entry_point: source.entry_point.clone(),
                stage: source.stage.clone(),
                hash: source_hash,
                reflection_data: reflection,
            };

            self.shader_cache.insert(source_hash, compiled_shader.clone());
            return Ok(Arc::new(compiled_shader));
        }
        drop(disk_cache);

        let spirv_code = self.compiler.lock().compile_glsl_to_spirv(&source)?;
        let module = self.create_shader_module(&spirv_code)?;
        let reflection = self.compiler.lock().reflect_spirv(&spirv_code)?;

        let compiled_shader = CompiledShader {
            spirv_code: spirv_code.clone(),
            module,
            entry_point: source.entry_point.clone(),
            stage: source.stage.clone(),
            hash: source_hash,
            reflection_data: reflection,
        };

        let cache_entry = ShaderCacheEntry {
            source_hash,
            spirv_code,
            compilation_time: std::time::SystemTime::now(),
            optimization_level: source.optimization_level,
        };

        self.disk_cache.write().insert(source_hash, cache_entry);
        self.shader_cache.insert(source_hash, compiled_shader.clone());

        tokio::spawn({
            let manager = Arc::new(self.clone());
            async move {
                let _ = manager.save_disk_cache();
            }
        });

        Ok(Arc::new(compiled_shader))
    }

    fn create_shader_module(&self, spirv_code: &[u32]) -> Result<vk::ShaderModule> {
        let create_info = vk::ShaderModuleCreateInfo::builder()
            .code(spirv_code);

        unsafe {
            self.device.logical_device().create_shader_module(&create_info, None)
                .map_err(|e| ShaderError::Pipeline(e.to_string()))
        }
    }

    pub async fn load_shader_from_file<P: AsRef<Path>>(
        &self,
        path: P,
        stage: ShaderStage,
        entry_point: &str
    ) -> Result<Arc<CompiledShader>> {
        let path = path.as_ref();
        let glsl_code = fs::read_to_string(path)
            .map_err(|e| ShaderError::Loading(format!("Failed to read shader file {}: {}", path.display(), e)))?;

        if self.hot_reload_enabled {
            let metadata = fs::metadata(path)
                .map_err(|e| ShaderError::Loading(format!("Failed to get file metadata: {}", e)))?;
            
            if let Ok(modified) = metadata.modified() {
                self.file_watchers.write().insert(path.to_path_buf(), modified);
            }
        }

        let source = ShaderSource {
            glsl_code,
            entry_point: entry_point.to_string(),
            stage,
            include_paths: vec![path.parent().unwrap_or(Path::new(".")).to_path_buf()],
            defines: HashMap::new(),
            optimization_level: if cfg!(debug_assertions) { 
                OptimizationLevel::Debug 
            } else { 
                OptimizationLevel::Performance 
            },
        };

        self.compile_shader(source).await
    }

    pub async fn create_graphics_pipeline(
        &self,
        vertex_shader: Arc<CompiledShader>,
        fragment_shader: Arc<CompiledShader>,
        render_pass: vk::RenderPass,
        layout: vk::PipelineLayout
    ) -> Result<vk::Pipeline> {
        let vertex_stage_info = vk::PipelineShaderStageCreateInfo::builder()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vertex_shader.module)
            .name(unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(b"main\0") });

        let fragment_stage_info = vk::PipelineShaderStageCreateInfo::builder()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(fragment_shader.module)
            .name(unsafe { std::ffi::CStr::from_bytes_with_nul_unchecked(b"main\0") });

        let shader_stages = [*vertex_stage_info, *fragment_stage_info];

        let vertex_input_info = vk::PipelineVertexInputStateCreateInfo::builder()
            .vertex_binding_descriptions(&[])
            .vertex_attribute_descriptions(&[]);

        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::builder()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
            .primitive_restart_enable(false);

        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
            min_depth: 0.0,
            max_depth: 1.0,
        };

        let scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D { width: 1920, height: 1080 },
        };

        let viewports = [viewport];
        let scissors = [scissor];

        let viewport_state = vk::PipelineViewportStateCreateInfo::builder()
            .viewports(&viewports)
            .scissors(&scissors);

        let rasterizer = vk::PipelineRasterizationStateCreateInfo::builder()
            .depth_clamp_enable(false)
            .rasterizer_discard_enable(false)
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0)
            .cull_mode(vk::CullModeFlags::BACK)
            .front_face(vk::FrontFace::CLOCKWISE)
            .depth_bias_enable(false);

        let multisampling = vk::PipelineMultisampleStateCreateInfo::builder()
            .sample_shading_enable(false)
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);

        let color_blend_attachment = vk::PipelineColorBlendAttachmentState::builder()
            .color_write_mask(vk::ColorComponentFlags::RGBA)
            .blend_enable(false);

        let color_blend_attachments = [*color_blend_attachment];
        let color_blending = vk::PipelineColorBlendStateCreateInfo::builder()
            .logic_op_enable(false)
            .logic_op(vk::LogicOp::COPY)
            .attachments(&color_blend_attachments)
            .blend_constants([0.0, 0.0, 0.0, 0.0]);

        let pipeline_info = vk::GraphicsPipelineCreateInfo::builder()
            .stages(&shader_stages)
            .vertex_input_state(&vertex_input_info)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&rasterizer)
            .multisample_state(&multisampling)
            .color_blend_state(&color_blending)
            .layout(layout)
            .render_pass(render_pass)
            .subpass(0);

        let pipelines = unsafe {
            self.device.logical_device().create_graphics_pipelines(
                vk::PipelineCache::null(),
                &[*pipeline_info],
                None
            ).map_err(|e| ShaderError::Pipeline(e.1.to_string()))?
        };

        Ok(pipelines[0])
    }

    pub async fn check_hot_reload(&self) -> Result<Vec<PathBuf>> {
        if !self.hot_reload_enabled {
            return Ok(Vec::new());
        }

        let mut changed_files = Vec::new();
        let mut watchers = self.file_watchers.write();

        for (path, last_modified) in watchers.iter_mut() {
            if let Ok(metadata) = fs::metadata(path) {
                if let Ok(modified) = metadata.modified() {
                    if modified > *last_modified {
                        *last_modified = modified;
                        changed_files.push(path.clone());
                    }
                }
            }
        }

        Ok(changed_files)
    }

    pub async fn reload_shader(&self, path: &Path, stage: ShaderStage, entry_point: &str) -> Result<Arc<CompiledShader>> {
        let old_shader_opt = {
            let watchers = self.file_watchers.read();
            if watchers.contains_key(path) {
                Some(())
            } else {
                None
            }
        };

        if old_shader_opt.is_some() {
            self.load_shader_from_file(path, stage, entry_point).await
        } else {
            Err(ShaderError::HotReload(format!("Shader not watched: {}", path.display())))
        }
    }

    pub async fn get_shader_by_hash(&self, hash: u64) -> Option<Arc<CompiledShader>> {
        self.shader_cache.get(&hash).map(|entry| Arc::new(entry.clone()))
    }

    pub fn cleanup_shader(&self, shader: &CompiledShader) {
        unsafe {
            self.device.logical_device().destroy_shader_module(shader.module, None);
        }
    }

    pub async fn shutdown(&self) -> Result<()> {
        self.save_disk_cache()?;

        for entry in self.shader_cache.iter() {
            unsafe {
                self.device.logical_device().destroy_shader_module(entry.module, None);
            }
        }

        self.shader_cache.clear();

        Ok(())
    }
}

impl Clone for ShaderManager {
    fn clone(&self) -> Self {
        Self {
            device: self.device.clone(),
            compiler: self.compiler.clone(),
            shader_cache: self.shader_cache.clone(),
            disk_cache: self.disk_cache.clone(),
            cache_path: self.cache_path.clone(),
            hot_reload_enabled: self.hot_reload_enabled,
            file_watchers: self.file_watchers.clone(),
        }
    }
}