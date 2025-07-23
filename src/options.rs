#[derive(Debug, Default)]
pub struct SystemJsTranspilerOptions {
    pub module_ids: bool,
    pub module_id: String,
    // get_module_id:
    pub module_root: String,
    pub allow_top_level_this: bool,
    pub system_global: String,
}
