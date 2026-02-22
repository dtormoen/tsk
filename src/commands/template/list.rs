use crate::assets::frontmatter::parse_frontmatter;
use crate::assets::{find_template, list_all_templates};
use crate::commands::Command;
use crate::context::AppContext;
use crate::display::print_columns;
use crate::repo_utils::find_repository_root;
use async_trait::async_trait;
use std::error::Error;
use std::path::Path;

pub struct TemplateListCommand;

#[async_trait]
impl Command for TemplateListCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        let project_root = find_repository_root(Path::new(".")).ok();

        // List all available templates
        let templates = list_all_templates(project_root.as_deref(), &ctx.tsk_env());

        if templates.is_empty() {
            println!("No templates available");
            return Ok(());
        }

        // Determine source and description for each template
        let mut rows = Vec::new();
        for template in &templates {
            let source = determine_template_source(template, project_root.as_deref(), ctx)?;
            let description = find_template(template, project_root.as_deref(), &ctx.tsk_env())
                .ok()
                .and_then(|content| parse_frontmatter(&content).description)
                .unwrap_or_default();
            rows.push(vec![template.to_string(), source, description]);
        }

        println!("Available Templates:\n");
        print_columns(&["Name", "Source", "Description"], &rows);

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
