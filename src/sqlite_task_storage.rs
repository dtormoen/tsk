use crate::task::{Task, TaskStatus};
use crate::task_storage::TaskStorage;
use chrono::DateTime;
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

fn path_to_string(path: &Path) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    path.to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("Path contains non-UTF-8 characters: {}", path.display()).into())
}

fn task_status_to_str(status: &TaskStatus) -> &'static str {
    match status {
        TaskStatus::Queued => "QUEUED",
        TaskStatus::Running => "RUNNING",
        TaskStatus::Failed => "FAILED",
        TaskStatus::Complete => "COMPLETE",
    }
}

fn str_to_task_status(s: &str) -> Result<TaskStatus, Box<dyn std::error::Error + Send + Sync>> {
    match s {
        "QUEUED" => Ok(TaskStatus::Queued),
        "RUNNING" => Ok(TaskStatus::Running),
        "FAILED" => Ok(TaskStatus::Failed),
        "COMPLETE" => Ok(TaskStatus::Complete),
        _ => Err(format!("Unknown task status: {s}").into()),
    }
}

fn row_to_task(row: &rusqlite::Row) -> rusqlite::Result<Task> {
    let status_str: String = row.get("status")?;
    let created_at_str: String = row.get("created_at")?;
    let started_at_str: Option<String> = row.get("started_at")?;
    let completed_at_str: Option<String> = row.get("completed_at")?;
    let repo_root_str: String = row.get("repo_root")?;
    let copied_repo_path_str: Option<String> = row.get("copied_repo_path")?;
    let is_interactive_int: i32 = row.get("is_interactive")?;

    Ok(Task {
        id: row.get("id")?,
        repo_root: PathBuf::from(repo_root_str),
        name: row.get("name")?,
        task_type: row.get("task_type")?,
        instructions_file: row.get("instructions_file")?,
        agent: row.get("agent")?,
        status: str_to_task_status(&status_str).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, e)
        })?,
        created_at: DateTime::parse_from_rfc3339(&created_at_str)
            .map(|dt| dt.with_timezone(&chrono::Local))
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    7,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
        started_at: started_at_str
            .map(|s| DateTime::parse_from_rfc3339(&s).map(|dt| dt.with_timezone(&chrono::Utc)))
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    8,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
        completed_at: completed_at_str
            .map(|s| DateTime::parse_from_rfc3339(&s).map(|dt| dt.with_timezone(&chrono::Utc)))
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    9,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?,
        branch_name: row.get("branch_name")?,
        error_message: row.get("error_message")?,
        source_commit: row.get("source_commit")?,
        source_branch: row.get("source_branch")?,
        stack: row.get("stack")?,
        project: row.get("project")?,
        copied_repo_path: copied_repo_path_str.map(PathBuf::from),
        is_interactive: is_interactive_int != 0,
        parent_id: row.get("parent_id")?,
    })
}

fn insert_task(
    conn: &Connection,
    task: &Task,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let repo_root = path_to_string(&task.repo_root)?;
    let copied_repo_path = task
        .copied_repo_path
        .as_ref()
        .map(|p| path_to_string(p))
        .transpose()?;
    conn.execute(
        "INSERT INTO tasks (id, repo_root, name, task_type, instructions_file, agent, status, created_at, started_at, completed_at, branch_name, error_message, source_commit, source_branch, stack, project, copied_repo_path, is_interactive, parent_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        rusqlite::params![
            task.id,
            repo_root,
            task.name,
            task.task_type,
            task.instructions_file,
            task.agent,
            task_status_to_str(&task.status),
            task.created_at.to_rfc3339(),
            task.started_at.map(|dt| dt.to_rfc3339()),
            task.completed_at.map(|dt| dt.to_rfc3339()),
            task.branch_name,
            task.error_message,
            task.source_commit,
            task.source_branch,
            task.stack,
            task.project,
            copied_repo_path,
            task.is_interactive as i32,
            task.parent_id,
        ],
    )?;
    Ok(())
}

/// Attempts to migrate tasks from a legacy `tasks.json` file into the SQLite database.
///
/// Migration runs only when:
/// - `tasks.json` exists in `data_dir`
/// - `tasks.json.bak` does NOT exist (prevents re-migration)
/// - The `tasks` table is empty
///
/// After successful migration, `tasks.json` is renamed to `tasks.json.bak`.
fn migrate_from_json(conn: &Connection, data_dir: &Path) {
    let json_path = data_dir.join("tasks.json");
    let bak_path = data_dir.join("tasks.json.bak");

    if !json_path.exists() {
        return;
    }

    if bak_path.exists() {
        return;
    }

    let count: i64 = match conn.query_row("SELECT COUNT(*) FROM tasks", [], |row| row.get(0)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Warning: failed to check tasks table during migration: {e}");
            return;
        }
    };
    if count > 0 {
        return;
    }

    let contents = match fs::read_to_string(&json_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Warning: failed to read tasks.json for migration: {e}");
            return;
        }
    };

    let tasks: Vec<Task> = match serde_json::from_str(&contents) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Warning: tasks.json contains invalid data, skipping migration: {e}");
            if let Err(rename_err) = fs::rename(&json_path, &bak_path) {
                eprintln!("Warning: failed to rename tasks.json to tasks.json.bak: {rename_err}");
            }
            return;
        }
    };

    if tasks.is_empty() {
        eprintln!("Migrated 0 tasks from tasks.json to tasks.db");
        if let Err(e) = fs::rename(&json_path, &bak_path) {
            eprintln!("Warning: failed to rename tasks.json to tasks.json.bak: {e}");
        }
        return;
    }

    // Safe: we have exclusive access during construction and transaction() requires &mut
    let tx = match conn.unchecked_transaction() {
        Ok(tx) => tx,
        Err(e) => {
            eprintln!("Warning: failed to begin migration transaction: {e}");
            return;
        }
    };

    for task in &tasks {
        if let Err(e) = insert_task(&tx, task) {
            eprintln!("Warning: failed to migrate task {}: {e}", task.id);
            return; // Transaction will be rolled back on drop
        }
    }

    if let Err(e) = tx.commit() {
        eprintln!("Warning: failed to commit migration transaction: {e}");
        return;
    }

    if let Err(e) = fs::rename(&json_path, &bak_path) {
        eprintln!("Warning: failed to rename tasks.json to tasks.json.bak: {e}");
        return;
    }

    eprintln!("Migrated {} tasks from tasks.json to tasks.db", tasks.len());
}

/// SQLite-backed implementation of `TaskStorage`.
///
/// Stores tasks in a SQLite database with WAL mode enabled for concurrent read performance.
/// All database operations are executed via `tokio::task::spawn_blocking` to avoid blocking
/// the async runtime.
pub struct SqliteTaskStorage {
    conn: Arc<StdMutex<Connection>>,
}

impl SqliteTaskStorage {
    /// Creates a new `SqliteTaskStorage`, opening or creating the database at `db_path`.
    ///
    /// Enables WAL journal mode and creates the `tasks` table and indexes if they don't exist.
    pub fn new(db_path: PathBuf) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY,
                repo_root TEXT NOT NULL,
                name TEXT NOT NULL,
                task_type TEXT NOT NULL,
                instructions_file TEXT NOT NULL,
                agent TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                started_at TEXT,
                completed_at TEXT,
                branch_name TEXT NOT NULL,
                error_message TEXT,
                source_commit TEXT NOT NULL,
                source_branch TEXT,
                stack TEXT NOT NULL,
                project TEXT NOT NULL,
                copied_repo_path TEXT,
                is_interactive INTEGER NOT NULL DEFAULT 0,
                parent_id TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
            CREATE INDEX IF NOT EXISTS idx_tasks_parent_id ON tasks(parent_id);",
        )?;

        if let Some(data_dir) = db_path.parent() {
            migrate_from_json(&conn, data_dir);
        }

        Ok(Self {
            conn: Arc::new(StdMutex::new(conn)),
        })
    }
}

#[async_trait::async_trait]
impl TaskStorage for SqliteTaskStorage {
    async fn add_task(&self, task: Task) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("Lock error: {e}"))?;
            insert_task(&conn, &task)
        })
        .await??;
        Ok(())
    }

    async fn get_task(
        &self,
        id: &str,
    ) -> Result<Option<Task>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = Arc::clone(&self.conn);
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("Lock error: {e}"))?;
            let mut stmt = conn.prepare("SELECT * FROM tasks WHERE id = ?1")?;
            let mut rows = stmt.query_map(rusqlite::params![id], row_to_task)?;
            match rows.next() {
                Some(row) => Ok(Some(row?)),
                None => Ok(None),
            }
        })
        .await?
    }

    async fn list_tasks(&self) -> Result<Vec<Task>, Box<dyn std::error::Error + Send + Sync>> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("Lock error: {e}"))?;
            let mut stmt = conn.prepare("SELECT * FROM tasks ORDER BY created_at")?;
            let tasks = stmt
                .query_map([], row_to_task)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(tasks)
        })
        .await?
    }

    async fn update_task(
        &self,
        task: Task,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("Lock error: {e}"))?;
            let repo_root = path_to_string(&task.repo_root)?;
            let copied_repo_path = task
                .copied_repo_path
                .as_ref()
                .map(|p| path_to_string(p))
                .transpose()?;
            let rows_affected = conn.execute(
                "UPDATE tasks SET repo_root = ?1, name = ?2, task_type = ?3, instructions_file = ?4, agent = ?5, status = ?6, created_at = ?7, started_at = ?8, completed_at = ?9, branch_name = ?10, error_message = ?11, source_commit = ?12, source_branch = ?13, stack = ?14, project = ?15, copied_repo_path = ?16, is_interactive = ?17, parent_id = ?18 WHERE id = ?19",
                rusqlite::params![
                    repo_root,
                    task.name,
                    task.task_type,
                    task.instructions_file,
                    task.agent,
                    task_status_to_str(&task.status),
                    task.created_at.to_rfc3339(),
                    task.started_at.map(|dt| dt.to_rfc3339()),
                    task.completed_at.map(|dt| dt.to_rfc3339()),
                    task.branch_name,
                    task.error_message,
                    task.source_commit,
                    task.source_branch,
                    task.stack,
                    task.project,
                    copied_repo_path,
                    task.is_interactive as i32,
                    task.parent_id,
                    task.id,
                ],
            )?;
            if rows_affected == 0 {
                return Err("Task not found".into());
            }
            Ok(())
        })
        .await?
    }

    async fn update_task_status(
        &self,
        id: &str,
        status: TaskStatus,
        started_at: Option<chrono::DateTime<chrono::Utc>>,
        completed_at: Option<chrono::DateTime<chrono::Utc>>,
        error_message: Option<String>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = Arc::clone(&self.conn);
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("Lock error: {e}"))?;
            let rows_affected = conn.execute(
                "UPDATE tasks SET status = ?1, started_at = COALESCE(?2, started_at), completed_at = COALESCE(?3, completed_at), error_message = COALESCE(?4, error_message) WHERE id = ?5",
                rusqlite::params![
                    task_status_to_str(&status),
                    started_at.map(|dt| dt.to_rfc3339()),
                    completed_at.map(|dt| dt.to_rfc3339()),
                    error_message,
                    id,
                ],
            )?;
            if rows_affected == 0 {
                return Err("Task not found".into());
            }
            Ok(())
        })
        .await?
    }

    async fn delete_task(&self, id: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conn = Arc::clone(&self.conn);
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("Lock error: {e}"))?;
            let rows_affected =
                conn.execute("DELETE FROM tasks WHERE id = ?1", rusqlite::params![id])?;
            if rows_affected == 0 {
                return Err("Task not found".into());
            }
            Ok(())
        })
        .await?
    }

    async fn delete_tasks_by_status(
        &self,
        statuses: Vec<TaskStatus>,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("Lock error: {e}"))?;
            let placeholders: Vec<String> = (1..=statuses.len()).map(|i| format!("?{i}")).collect();
            let sql = format!(
                "DELETE FROM tasks WHERE status IN ({})",
                placeholders.join(", ")
            );
            let params: Vec<String> = statuses
                .iter()
                .map(|s| task_status_to_str(s).to_string())
                .collect();
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let rows_affected = conn.execute(&sql, param_refs.as_slice())?;
            Ok(rows_affected)
        })
        .await?
    }
}
