use serde::Serialize;
use serde_json::Value;

use crate::edition::Edition;
use crate::error::{Chat2DbError, Result};
use crate::installer::SystemPaths;
use crate::runtime::{RuntimeRequest, runtime_client};
use crate::runtime_client::{
    DatasourceConnectionTestRequest, DatasourceCreateRequest, DatasourceUpdateRequest,
    RuntimeCapability, SqlQueryRequest,
};

#[derive(Debug, Clone)]
pub struct DirectRequest {
    pub edition: Edition,
    pub paths: SystemPaths,
}

impl DirectRequest {
    pub fn production(edition: Edition) -> Self {
        Self {
            edition,
            paths: SystemPaths::production(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DirectOutcome {
    pub command: &'static str,
    pub edition: Edition,
    pub data: Value,
}

pub fn list_datasources(
    request: DirectRequest,
    page_no: u32,
    page_size: u32,
    refresh: bool,
) -> Result<DirectOutcome> {
    let client = runtime_client(
        &RuntimeRequest {
            edition: request.edition,
            paths: request.paths,
        },
        RuntimeCapability::DatasourceList,
    )?;
    Ok(DirectOutcome {
        command: "db_datasources",
        edition: request.edition,
        data: client.datasources(page_no, page_size, refresh)?,
    })
}

pub fn get_datasource(request: DirectRequest, data_source_id: u64) -> Result<DirectOutcome> {
    let client = runtime_client(
        &RuntimeRequest {
            edition: request.edition,
            paths: request.paths,
        },
        RuntimeCapability::DatasourceGet,
    )?;
    Ok(DirectOutcome {
        command: "db_datasource",
        edition: request.edition,
        data: client.datasource_get(data_source_id)?,
    })
}

pub fn create_datasource(
    request: DirectRequest,
    create_request: DatasourceCreateRequest,
) -> Result<DirectOutcome> {
    let client = runtime_client(
        &RuntimeRequest {
            edition: request.edition,
            paths: request.paths,
        },
        RuntimeCapability::DatasourceCreate,
    )?;
    Ok(DirectOutcome {
        command: "db_datasource_create",
        edition: request.edition,
        data: client.datasource_create(create_request)?,
    })
}

pub fn update_datasource(
    request: DirectRequest,
    update_request: DatasourceUpdateRequest,
) -> Result<DirectOutcome> {
    let client = runtime_client(
        &RuntimeRequest {
            edition: request.edition,
            paths: request.paths,
        },
        RuntimeCapability::DatasourceUpdate,
    )?;
    Ok(DirectOutcome {
        command: "db_datasource_update",
        edition: request.edition,
        data: client.datasource_update(update_request)?,
    })
}

pub fn delete_datasource(request: DirectRequest, data_source_id: u64) -> Result<DirectOutcome> {
    let client = runtime_client(
        &RuntimeRequest {
            edition: request.edition,
            paths: request.paths,
        },
        RuntimeCapability::DatasourceDelete,
    )?;
    Ok(DirectOutcome {
        command: "db_datasource_delete",
        edition: request.edition,
        data: client.datasource_delete(data_source_id)?,
    })
}

pub fn connection_test(
    request: DirectRequest,
    connection_request: DatasourceConnectionTestRequest,
) -> Result<DirectOutcome> {
    let required_capability = if connection_request.data_source_id.is_some() {
        RuntimeCapability::DatasourceGet
    } else {
        RuntimeCapability::DatasourceCreate
    };
    let client = runtime_client(
        &RuntimeRequest {
            edition: request.edition,
            paths: request.paths,
        },
        required_capability,
    )?;
    let data = client.datasource_connection_test(connection_request)?;
    if data
        .get("canConnect")
        .and_then(Value::as_bool)
        .is_some_and(|can_connect| !can_connect)
    {
        let message = data
            .get("errorMessage")
            .and_then(Value::as_str)
            .or_else(|| data.get("errorCode").and_then(Value::as_str))
            .unwrap_or("connection test failed")
            .to_string();
        return Err(Chat2DbError::DatasourceConnectionFailed { message });
    }
    Ok(DirectOutcome {
        command: "db_connection_test",
        edition: request.edition,
        data,
    })
}

pub fn list_databases(
    request: DirectRequest,
    data_source_id: u64,
    refresh: bool,
) -> Result<DirectOutcome> {
    let client = runtime_client(
        &RuntimeRequest {
            edition: request.edition,
            paths: request.paths,
        },
        RuntimeCapability::MetadataDatabaseList,
    )?;
    Ok(DirectOutcome {
        command: "db_databases",
        edition: request.edition,
        data: client.databases(data_source_id, refresh)?,
    })
}

pub fn list_schemas(
    request: DirectRequest,
    data_source_id: u64,
    database_name: String,
    refresh: bool,
) -> Result<DirectOutcome> {
    let client = runtime_client(
        &RuntimeRequest {
            edition: request.edition,
            paths: request.paths,
        },
        RuntimeCapability::MetadataSchemaList,
    )?;
    Ok(DirectOutcome {
        command: "db_schemas",
        edition: request.edition,
        data: client.schemas(data_source_id, database_name, refresh)?,
    })
}

pub fn list_tables(
    request: DirectRequest,
    data_source_id: u64,
    database_name: String,
    schema_name: Option<String>,
    page_no: u32,
    page_size: u32,
    refresh: bool,
) -> Result<DirectOutcome> {
    let client = runtime_client(
        &RuntimeRequest {
            edition: request.edition,
            paths: request.paths,
        },
        RuntimeCapability::MetadataTableList,
    )?;
    Ok(DirectOutcome {
        command: "db_tables",
        edition: request.edition,
        data: client.tables(
            data_source_id,
            database_name,
            schema_name,
            page_no,
            page_size,
            refresh,
        )?,
    })
}

pub fn table_detail(
    request: DirectRequest,
    data_source_id: u64,
    database_name: String,
    schema_name: Option<String>,
    table_name: String,
) -> Result<DirectOutcome> {
    let client = runtime_client(
        &RuntimeRequest {
            edition: request.edition,
            paths: request.paths,
        },
        RuntimeCapability::MetadataTableDetail,
    )?;
    Ok(DirectOutcome {
        command: "db_table",
        edition: request.edition,
        data: client.table_detail(data_source_id, database_name, schema_name, table_name)?,
    })
}

pub fn sql_query(request: DirectRequest, sql_request: SqlQueryRequest) -> Result<DirectOutcome> {
    let client = runtime_client(
        &RuntimeRequest {
            edition: request.edition,
            paths: request.paths,
        },
        RuntimeCapability::SqlQuery,
    )?;
    Ok(DirectOutcome {
        command: "sql_query",
        edition: request.edition,
        data: client.sql_query(sql_request)?,
    })
}
