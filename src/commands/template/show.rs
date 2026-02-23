use crate::assets::find_template;
use crate::commands::Command;
use crate::context::AppContext;
use crate::repo_utils::find_repository_root;
use async_trait::async_trait;
use std::error::Error;
use std::path::Path;

pub struct TemplateShowCommand {
    pub name: String,
}

#[async_trait]
impl Command for TemplateShowCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        let project_root = find_repository_root(Path::new(".")).ok();
        let content = find_template(&self.name, project_root.as_deref(), &ctx.tsk_env())?;
        print!("{content}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::AppContext;

    #[tokio::test]
    async fn test_show_embedded_template() {
        let ctx = AppContext::builder().build();
        let cmd = TemplateShowCommand {
            name: "feat".to_string(),
        };
        // Should succeed without error for an embedded template
        cmd.execute(&ctx).await.unwrap();
    }

    #[tokio::test]
    async fn test_show_nonexistent_template() {
        let ctx = AppContext::builder().build();
        let cmd = TemplateShowCommand {
            name: "nonexistent-xyz-template".to_string(),
        };
        let result = cmd.execute(&ctx).await;
        assert!(result.is_err());
    }
}
