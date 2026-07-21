use crate::cli::{DbConnectionTestArgs, DbOp};
use crate::direct::{
    DirectRequest, connection_test, create_datasource, delete_datasource, get_datasource,
    list_databases, list_datasources, list_schemas, list_tables, table_detail, update_datasource,
};
use crate::error::{Chat2DbError, Result};
use crate::output::Outcome;
use crate::runtime_client::{
    DatasourceConnectionTestRequest, DatasourceCreateRequest, DatasourceUpdateRequest,
};

const CONNECTION_TEST_HOST_MODE_DB_TYPES: &[&str] = &[
    "MYSQL",
    "MARIADB",
    "TIDB",
    "DORIS",
    "STARROCKS",
    "POSTGRESQL",
    "POSTGRES",
    "COCKROACHDB",
    "GAUSSDB",
    "OPENGAUSS",
    "SQLSERVER",
    "ORACLE",
    "DB2",
    "DM",
    "KINGBASE",
    "CLICKHOUSE",
    "PRESTO",
    "HIVE",
    "OCEANBASE",
    "OCEANBASE_ORACLE",
    "MONGODB",
    "REDIS",
    "H2",
    "REDSHIFT",
    "KYLIN",
    "OSCAR",
    "SUNDB",
    "XUGUDB",
    "TDENGINE",
];

pub fn run_db_op(op: DbOp) -> Result<Vec<Outcome>> {
    match op {
        DbOp::Datasources(a) => list_datasources(
            DirectRequest::production(a.edition),
            a.page_no,
            a.page_size,
            a.refresh,
        )
        .map(|outcome| vec![Outcome::Direct(outcome)]),
        DbOp::Datasource(a) => {
            get_datasource(DirectRequest::production(a.edition), a.data_source_id)
                .map(|outcome| vec![Outcome::Direct(outcome)])
        }
        DbOp::ConnectionTest(a) => {
            let edition = a.edition;
            connection_test(
                DirectRequest::production(edition),
                connection_test_request(a)?,
            )
            .map(|outcome| vec![Outcome::Direct(outcome)])
        }
        DbOp::DatasourceCreate(a) => create_datasource(
            DirectRequest::production(a.edition),
            datasource_create_request(a)?,
        )
        .map(|outcome| vec![Outcome::Direct(outcome)]),
        DbOp::DatasourceUpdate(a) => update_datasource(
            DirectRequest::production(a.edition),
            DatasourceUpdateRequest {
                data_source_id: a.data_source_id,
                alias: a.alias,
                db_type: a.db_type,
                url: a.url,
                user: a.user,
                password: a.password,
                environment_id: a.environment_id,
                host: a.host,
                port: a.port,
                database: a.database,
                service_name: a.service_name,
                service_type: a.service_type,
                storage_type: a.storage_type,
                space_id: a.space_id,
            },
        )
        .map(|outcome| vec![Outcome::Direct(outcome)]),
        DbOp::DatasourceDelete(a) => {
            if !a.yes {
                return Err(Chat2DbError::ConfirmationRequired {
                    action: "delete datasource",
                });
            }
            delete_datasource(DirectRequest::production(a.edition), a.data_source_id)
                .map(|outcome| vec![Outcome::Direct(outcome)])
        }
        DbOp::Databases(a) => list_databases(
            DirectRequest::production(a.edition),
            a.data_source_id,
            a.refresh,
        )
        .map(|outcome| vec![Outcome::Direct(outcome)]),
        DbOp::Schemas(a) => list_schemas(
            DirectRequest::production(a.edition),
            a.data_source_id,
            a.database,
            a.refresh,
        )
        .map(|outcome| vec![Outcome::Direct(outcome)]),
        DbOp::Tables(a) => list_tables(
            DirectRequest::production(a.edition),
            a.data_source_id,
            a.database,
            a.schema,
            a.page_no,
            a.page_size,
            a.refresh,
        )
        .map(|outcome| vec![Outcome::Direct(outcome)]),
        DbOp::Table(a) => table_detail(
            DirectRequest::production(a.edition),
            a.data_source_id,
            a.database,
            a.schema,
            a.table,
        )
        .map(|outcome| vec![Outcome::Direct(outcome)]),
    }
}

fn connection_test_request(a: DbConnectionTestArgs) -> Result<DatasourceConnectionTestRequest> {
    let temporary_fields_present = a.db_type.is_some()
        || a.url.is_some()
        || a.host.is_some()
        || a.port.is_some()
        || a.database.is_some()
        || a.user.is_some()
        || a.password.is_some()
        || a.service_name.is_some()
        || a.service_type.is_some();
    if let Some(data_source_id) = a.data_source_id {
        if temporary_fields_present {
            return Err(Chat2DbError::InvalidConnectionTestArgs {
                message: "--data-source-id cannot be combined with temporary connection parameters"
                    .to_string(),
            });
        }
        return Ok(DatasourceConnectionTestRequest {
            data_source_id: Some(data_source_id),
            db_type: None,
            url: None,
            host: None,
            port: None,
            database: None,
            user: None,
            password: None,
            service_name: None,
            service_type: None,
        });
    }

    let db_type = required_connection_test_arg(a.db_type, "--db-type")?;
    let user = required_connection_test_arg(a.user, "--user")?;
    let password = required_connection_test_arg(a.password, "--password")?;
    let (url, host, port, database) = match (a.url, a.host, a.port, a.database) {
        (Some(url), None, None, None) if !url.trim().is_empty() => (Some(url), None, None, None),
        (Some(_), None, None, None) => {
            return Err(Chat2DbError::InvalidConnectionTestArgs {
                message: "--url cannot be empty".to_string(),
            });
        }
        (Some(_), _, _, _) => {
            return Err(Chat2DbError::InvalidConnectionTestArgs {
                message: "--url cannot be combined with --host, --port, or --database".to_string(),
            });
        }
        (None, Some(host), Some(port), Some(database))
            if !host.trim().is_empty()
                && !port.trim().is_empty()
                && !database.trim().is_empty() =>
        {
            validate_connection_test_host_mode_db_type(&db_type)?;
            (None, Some(host), Some(port), Some(database))
        }
        (None, Some(_), Some(_), Some(_)) => {
            return Err(Chat2DbError::InvalidConnectionTestArgs {
                message: "host mode requires non-empty --host, --port, and --database".to_string(),
            });
        }
        _ => {
            return Err(Chat2DbError::InvalidConnectionTestArgs {
                message: "temporary mode requires either --url or --host --port --database"
                    .to_string(),
            });
        }
    };

    Ok(DatasourceConnectionTestRequest {
        data_source_id: None,
        db_type: Some(db_type),
        url,
        host,
        port,
        database: database.clone(),
        user: Some(user),
        password: Some(password),
        service_name: a.service_name.or(database),
        service_type: a.service_type,
    })
}

fn required_connection_test_arg(value: Option<String>, name: &'static str) -> Result<String> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value),
        Some(_) => Err(Chat2DbError::InvalidConnectionTestArgs {
            message: format!("{name} cannot be empty"),
        }),
        None => Err(Chat2DbError::InvalidConnectionTestArgs {
            message: format!("temporary mode requires {name}"),
        }),
    }
}

fn datasource_create_request(
    a: crate::cli::DbDatasourceCreateArgs,
) -> Result<DatasourceCreateRequest> {
    let has_url = a.url.as_deref().is_some_and(|url| !url.trim().is_empty());
    let host_fields_present = a.host.is_some() || a.port.is_some() || a.database.is_some();
    let url = match (&a.url, has_url, host_fields_present) {
        (Some(_), false, _) => {
            return Err(Chat2DbError::InvalidDatasourceCreateArgs {
                message: "--url cannot be empty".to_string(),
            });
        }
        (Some(_), true, true) => {
            return Err(Chat2DbError::InvalidDatasourceCreateArgs {
                message: "--url cannot be combined with --host, --port, or --database".to_string(),
            });
        }
        (None, _, false) => {
            return Err(Chat2DbError::InvalidDatasourceCreateArgs {
                message: "datasource-create requires either --url or --host --port --database"
                    .to_string(),
            });
        }
        (None, _, true) => {
            let host = non_empty_create_host_field(a.host.as_deref());
            let port = non_empty_create_host_field(a.port.as_deref());
            let database = non_empty_create_host_field(a.database.as_deref());
            if host.is_none() || port.is_none() || database.is_none() {
                return Err(Chat2DbError::InvalidDatasourceCreateArgs {
                    message: "host mode requires non-empty --host, --port, and --database"
                        .to_string(),
                });
            }
            let normalized = a.db_type.to_ascii_uppercase();
            if !CONNECTION_TEST_HOST_MODE_DB_TYPES.contains(&normalized.as_str()) {
                return Err(Chat2DbError::InvalidDatasourceCreateArgs {
                    message: format!(
                        "--host/--port/--database URL construction is not supported for {normalized}; pass --url instead. Supported host-mode db types: {}",
                        CONNECTION_TEST_HOST_MODE_DB_TYPES.join(", ")
                    ),
                });
            }
            Some(connection_url(
                &normalized,
                host.unwrap(),
                port.unwrap(),
                database.unwrap(),
            )?)
        }
        (Some(url), true, false) => Some(url.clone()),
    };
    Ok(DatasourceCreateRequest {
        alias: a.alias,
        db_type: a.db_type,
        url,
        user: a.user,
        password: a.password,
        environment_id: a.environment_id,
        host: a.host,
        port: a.port,
        database: a.database,
        service_name: a.service_name,
        service_type: a.service_type,
        storage_type: a.storage_type,
        space_id: a.space_id,
    })
}

fn non_empty_create_host_field(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn connection_url(db_type: &str, host: &str, port: &str, database: &str) -> Result<String> {
    let url = match db_type {
        "MYSQL" => format!("jdbc:mysql://{host}:{port}/{database}"),
        "MARIADB" => format!("jdbc:mariadb://{host}:{port}/{database}"),
        "TIDB" | "DORIS" | "STARROCKS" => {
            format!("jdbc:mysql://{host}:{port}/{database}")
        }
        "POSTGRESQL" | "POSTGRES" | "COCKROACHDB" | "GAUSSDB" => {
            format!("jdbc:postgresql://{host}:{port}/{database}")
        }
        "OPENGAUSS" => format!("jdbc:opengauss://{host}:{port}/{database}"),
        "SQLSERVER" => format!("jdbc:sqlserver://{host}:{port};database={database}"),
        "ORACLE" => format!("jdbc:oracle:thin:@{host}:{port}:{database}"),
        "DB2" => format!("jdbc:db2://{host}:{port}/{database}"),
        "DM" => format!("jdbc:dm://{host}:{port}/{database}"),
        "KINGBASE" => format!("jdbc:kingbase8://{host}:{port}/{database}"),
        "CLICKHOUSE" => format!("jdbc:clickhouse://{host}:{port}/{database}"),
        "PRESTO" => format!("jdbc:presto://{host}:{port}/{database}"),
        "HIVE" => format!("jdbc:hive2://{host}:{port}/{database}"),
        "OCEANBASE" | "OCEANBASE_ORACLE" => {
            format!("jdbc:oceanbase://{host}:{port}/{database}")
        }
        "MONGODB" => format!("jdbc:mongodb://{host}:{port}/{database}"),
        "REDIS" => format!("jdbc:redis://{host}:{port}/{database}"),
        "H2" => format!("jdbc:h2:tcp://{host}:{port}/{database}"),
        "REDSHIFT" => format!("jdbc:redshift://{host}:{port}/{database}"),
        "KYLIN" => format!("jdbc:kylin://{host}:{port}/{database}"),
        "OSCAR" => format!("jdbc:oscar://{host}:{port}/{database}"),
        "SUNDB" => format!("jdbc:sundb://{host}:{port}/{database}"),
        "XUGUDB" => format!("jdbc:xugu://{host}:{port}/{database}"),
        "TDENGINE" => format!("jdbc:TAOS-RS://{host}:{port}/{database}"),
        _ => {
            return Err(Chat2DbError::InvalidDatasourceCreateArgs {
                message: format!(
                    "--host/--port/--database URL construction is not supported for {db_type}; pass --url instead"
                ),
            });
        }
    };
    Ok(url)
}

fn validate_connection_test_host_mode_db_type(db_type: &str) -> Result<()> {
    let normalized = db_type.to_ascii_uppercase();
    if CONNECTION_TEST_HOST_MODE_DB_TYPES.contains(&normalized.as_str()) {
        return Ok(());
    }
    Err(Chat2DbError::InvalidConnectionTestArgs {
        message: format!(
            "--host/--port/--database URL construction is not supported for {normalized}; pass --url instead. Supported host-mode db types: {}",
            CONNECTION_TEST_HOST_MODE_DB_TYPES.join(", ")
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edition::Edition;

    fn base_args() -> DbConnectionTestArgs {
        DbConnectionTestArgs {
            edition: Edition::Pro,
            data_source_id: None,
            db_type: None,
            url: None,
            host: None,
            port: None,
            database: None,
            user: None,
            password: None,
            service_name: None,
            service_type: None,
            json: true,
        }
    }

    #[test]
    fn existing_datasource_mode_rejects_temporary_fields() {
        let mut args = base_args();
        args.data_source_id = Some(195652);
        args.host = Some("localhost".to_string());

        let err = connection_test_request(args).unwrap_err();

        assert_eq!(err.code(), "invalid_connection_test_args");
        assert!(
            err.to_string()
                .contains("--data-source-id cannot be combined")
        );
    }

    #[test]
    fn temporary_url_mode_keeps_url_without_host_fields() {
        let mut args = base_args();
        args.db_type = Some("MYSQL".to_string());
        args.url = Some("jdbc:mysql://localhost:3306/test".to_string());
        args.user = Some("root".to_string());
        args.password = Some("secret".to_string());

        let request = connection_test_request(args).unwrap();

        assert_eq!(request.data_source_id, None);
        assert_eq!(
            request.url.as_deref(),
            Some("jdbc:mysql://localhost:3306/test")
        );
        assert_eq!(request.host, None);
        assert_eq!(request.password.as_deref(), Some("secret"));
    }

    #[test]
    fn temporary_host_mode_defaults_service_name_to_database() {
        let mut args = base_args();
        args.db_type = Some("POSTGRESQL".to_string());
        args.host = Some("localhost".to_string());
        args.port = Some("5432".to_string());
        args.database = Some("postgres".to_string());
        args.user = Some("postgres".to_string());
        args.password = Some("secret".to_string());

        let request = connection_test_request(args).unwrap();

        assert_eq!(request.host.as_deref(), Some("localhost"));
        assert_eq!(request.service_name.as_deref(), Some("postgres"));
    }

    #[test]
    fn temporary_host_mode_rejects_unknown_db_type() {
        let mut args = base_args();
        args.db_type = Some("UNKNOWNDB".to_string());
        args.host = Some("localhost".to_string());
        args.port = Some("9999".to_string());
        args.database = Some("test".to_string());
        args.user = Some("u".to_string());
        args.password = Some("secret".to_string());

        let err = connection_test_request(args).unwrap_err();

        assert_eq!(err.code(), "invalid_connection_test_args");
        assert!(err.to_string().contains("pass --url instead"));
    }

    #[test]
    fn temporary_mode_rejects_empty_password() {
        let mut args = base_args();
        args.db_type = Some("MYSQL".to_string());
        args.url = Some("jdbc:mysql://localhost:3306/test".to_string());
        args.user = Some("root".to_string());
        args.password = Some(String::new());

        let err = connection_test_request(args).unwrap_err();

        assert_eq!(err.code(), "invalid_connection_test_args");
        assert!(err.to_string().contains("--password cannot be empty"));
    }

    fn base_create_args() -> crate::cli::DbDatasourceCreateArgs {
        crate::cli::DbDatasourceCreateArgs {
            edition: Edition::Pro,
            alias: None,
            db_type: "MYSQL".to_string(),
            url: None,
            user: Some("root".to_string()),
            password: "secret".to_string(),
            environment_id: 1,
            host: None,
            port: None,
            database: None,
            service_name: None,
            service_type: None,
            storage_type: None,
            space_id: None,
            json: true,
        }
    }

    #[test]
    fn datasource_create_accepts_host_mode() {
        let mut args = base_create_args();
        args.host = Some("localhost".to_string());
        args.port = Some("3306".to_string());
        args.database = Some("app".to_string());

        let request = datasource_create_request(args).unwrap();

        assert_eq!(
            request.url.as_deref(),
            Some("jdbc:mysql://localhost:3306/app")
        );
        assert_eq!(request.host.as_deref(), Some("localhost"));
        assert_eq!(request.port.as_deref(), Some("3306"));
        assert_eq!(request.database.as_deref(), Some("app"));
    }

    #[test]
    fn datasource_create_host_mode_builds_postgres_url() {
        let mut args = base_create_args();
        args.db_type = "POSTGRES".to_string();
        args.host = Some("localhost".to_string());
        args.port = Some("5432".to_string());
        args.database = Some("postgres".to_string());

        let request = datasource_create_request(args).unwrap();

        assert_eq!(
            request.url.as_deref(),
            Some("jdbc:postgresql://localhost:5432/postgres")
        );
    }

    #[test]
    fn datasource_create_host_mode_builds_expected_jdbc_urls() {
        let expected_urls = [
            ("MYSQL", "jdbc:mysql://localhost:3306/app"),
            ("MARIADB", "jdbc:mariadb://localhost:3306/app"),
            ("TIDB", "jdbc:mysql://localhost:3306/app"),
            ("DORIS", "jdbc:mysql://localhost:3306/app"),
            ("STARROCKS", "jdbc:mysql://localhost:3306/app"),
            ("POSTGRESQL", "jdbc:postgresql://localhost:3306/app"),
            ("POSTGRES", "jdbc:postgresql://localhost:3306/app"),
            ("COCKROACHDB", "jdbc:postgresql://localhost:3306/app"),
            ("GAUSSDB", "jdbc:postgresql://localhost:3306/app"),
            ("OPENGAUSS", "jdbc:opengauss://localhost:3306/app"),
            ("SQLSERVER", "jdbc:sqlserver://localhost:3306;database=app"),
            ("ORACLE", "jdbc:oracle:thin:@localhost:3306:app"),
            ("DB2", "jdbc:db2://localhost:3306/app"),
            ("DM", "jdbc:dm://localhost:3306/app"),
            ("KINGBASE", "jdbc:kingbase8://localhost:3306/app"),
            ("CLICKHOUSE", "jdbc:clickhouse://localhost:3306/app"),
            ("PRESTO", "jdbc:presto://localhost:3306/app"),
            ("HIVE", "jdbc:hive2://localhost:3306/app"),
            ("OCEANBASE", "jdbc:oceanbase://localhost:3306/app"),
            ("OCEANBASE_ORACLE", "jdbc:oceanbase://localhost:3306/app"),
            ("MONGODB", "jdbc:mongodb://localhost:3306/app"),
            ("REDIS", "jdbc:redis://localhost:3306/app"),
            ("H2", "jdbc:h2:tcp://localhost:3306/app"),
            ("REDSHIFT", "jdbc:redshift://localhost:3306/app"),
            ("KYLIN", "jdbc:kylin://localhost:3306/app"),
            ("OSCAR", "jdbc:oscar://localhost:3306/app"),
            ("SUNDB", "jdbc:sundb://localhost:3306/app"),
            ("XUGUDB", "jdbc:xugu://localhost:3306/app"),
            ("TDENGINE", "jdbc:TAOS-RS://localhost:3306/app"),
        ];

        for (db_type, expected_url) in expected_urls {
            let mut args = base_create_args();
            args.db_type = db_type.to_string();
            args.host = Some("localhost".to_string());
            args.port = Some("3306".to_string());
            args.database = Some("app".to_string());

            let request = datasource_create_request(args).unwrap();

            assert_eq!(request.url.as_deref(), Some(expected_url), "{db_type}");
        }
    }

    #[test]
    fn datasource_create_allows_empty_password() {
        let mut args = base_create_args();
        args.url = Some("jdbc:mysql://localhost:3306/app".to_string());
        args.password = String::new();

        let request = datasource_create_request(args).unwrap();

        assert_eq!(request.password, "");
        assert_eq!(
            request.url.as_deref(),
            Some("jdbc:mysql://localhost:3306/app")
        );
    }

    #[test]
    fn datasource_create_rejects_url_and_host_mode_mix() {
        let mut args = base_create_args();
        args.url = Some("jdbc:mysql://localhost:3306/app".to_string());
        args.host = Some("localhost".to_string());

        let err = datasource_create_request(args).unwrap_err();

        assert_eq!(err.code(), "invalid_datasource_create_args");
        assert!(err.to_string().contains("--url cannot be combined"));
    }

    #[test]
    fn datasource_create_rejects_blank_url() {
        let mut args = base_create_args();
        args.url = Some("   ".to_string());

        let err = datasource_create_request(args).unwrap_err();

        assert_eq!(err.code(), "invalid_datasource_create_args");
        assert!(err.to_string().contains("--url cannot be empty"));
    }
}
