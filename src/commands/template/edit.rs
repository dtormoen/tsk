use crate::assets::{find_template, find_template_path};
use crate::commands::Command;
use crate::context::AppContext;
use crate::repo_utils::find_repository_root;
use async_trait::async_trait;
use std::error::Error;
use std::path::Path;

pub struct TemplateEditCommand {
    pub name: String,
}

#[async_trait]
impl Command for TemplateEditCommand {
    async fn execute(&self, ctx: &AppContext) -> Result<(), Box<dyn Error>> {
        let project_root = find_repository_root(Path::new(".")).ok();
        let tsk_env = ctx.tsk_env();

        let path = match find_template_path(&self.name, project_root.as_deref(), &tsk_env) {
            Some(path) => path,
            None => {
                // Template is embedded-only; copy to user templates dir
                let content = find_template(&self.name, project_root.as_deref(), &tsk_env)?;
                let user_templates_dir = tsk_env.config_dir().join("templates");
                std::fs::create_dir_all(&user_templates_dir)?;
                let dest = user_templates_dir.join(format!("{}.md", self.name));
                std::fs::write(&dest, &content)?;
                eprintln!(
                    "Copied built-in template '{}' to {} for editing",
                    self.name,
                    dest.display()
                );
                dest
            }
        };

        let editor = tsk_env.editor();
        let status = std::process::Command::new(editor).arg(&path).status()?;

        if !status.success() {
            return Err(format!("Editor '{}' exited with non-zero status", editor).into());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::tsk_env::TskEnv;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_edit_copies_embedded_template_to_user_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let temp_path = temp_dir.path();

        let tsk_env = Arc::new(
            TskEnv::builder()
                .with_data_dir(temp_path.join("data"))
                .with_runtime_dir(temp_path.join("runtime"))
                .with_config_dir(temp_path.join("config"))
                .with_claude_config_dir(temp_path.join("claude"))
                .with_editor("true".to_string())
                .build()
                .unwrap(),
        );
        tsk_env.ensure_directories().unwrap();

        let ctx = AppContext::builder().with_tsk_env(tsk_env).build();
        let cmd = TemplateEditCommand {
            name: "ack".to_string(),
        };

        cmd.execute(&ctx).await.unwrap();

        // Verify the embedded template was copied to the user templates dir
        let copied_path = temp_path
            .join("config")
            .join("tsk")
            .join("templates")
            .join("ack.md");
        assert!(copied_path.exists());
        let content = std::fs::read_to_string(&copied_path).unwrap();
        assert!(!content.is_empty());
    }
}
