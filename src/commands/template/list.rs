use crate::assets::{AssetManager, layered::LayeredAssetManager};
use crate::commands::Command;
use crate::context::AppContext;
use crate::repo_utils::find_repository_root;
use async_trait::async_trait;
use std::error::Error;
use std::path::Path;
use tabled::settings::Style;
use tabled::{Table, Tabled};

pub struct TemplateListCommand;

#[derive(Tabled)]
struct TemplateRow {
    #[tabled(rename = "Name")]
    name: String,
    #[tabled(rename = "Source")]
    source: String,
}

#[async_trait]
impl Command for TemplateListCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        let project_root = find_repository_root(Path::new(".")).ok();

        // Create asset manager on-demand
        let asset_manager =
            LayeredAssetManager::new_with_standard_layers(project_root.as_deref(), &ctx.tsk_env());

        // List all available templates
        let templates = asset_manager.list_templates();

        if templates.is_empty() {
            println!("No templates available");
            return Ok(());
        }

        // Determine source for each template
        let mut rows = Vec::new();
        for template in &templates {
            let source = determine_template_source(template, project_root.as_deref(), ctx)?;
            rows.push(TemplateRow {
                name: template.to_string(),
                source,
            });
        }

        let table = Table::new(rows).with(Style::modern()).to_string();
        println!("Available Templates:");
        println!("{table}");

        // Print additional information
        println!("\nTemplate locations (in priority order):");
        if let Some(root) = &project_root {
            println!("  1. Project: {}/.tsk/templates/", root.display());
        }
        println!(
            "  2. User: {}/templates/",
            ctx.tsk_env().config_dir().display()
        );
        println!("  3. Built-in: Embedded in TSK binary");

        Ok(())
    }
}

fn determine_template_source(
    template_name: &str,
    project_root: Option<&Path>,
    ctx: &AppContext,
) -> Result<String, Box<dyn Error>> {
    // Check project level first
    if let Some(root) = project_root {
        let project_template = root
            .join(".tsk")
            .join("templates")
            .join(format!("{template_name}.md"));
        if project_template.exists() {
            return Ok("Project".to_string());
        }
    }

    // Check user level
    let user_template = ctx
        .tsk_env()
        .config_dir()
        .join("templates")
        .join(format!("{template_name}.md"));
    if user_template.exists() {
        return Ok("User".to_string());
    }

    // Must be built-in
    Ok("Built-in".to_string())
}
