// SPDX-License-Identifier: AGPL-3.0-or-later OR LicenseRef-Mediasoft-Commercial
// Copyright (C) 2026 Mediasoft & Cie S.A.
//
// This file is part of ai-ontology.com.
// Dual-licensed: AGPL-3.0-or-later OR a commercial license
// from Mediasoft & Cie S.A. See LICENSE and LICENSE-COMMERCIAL.md.

//! The database connectors against real engines. A test runs when its
//! environment variable names an engine seeded with
//! `tests/fixtures/live/<engine>.sql`, and is skipped otherwise — unless
//! `ONTOLOGY_TEST_ENGINES=required` (the CI job `connectors`), where a
//! missing engine fails.
//!
//! - `ONTOLOGY_TEST_POSTGRES` / `ONTOLOGY_TEST_SQLSERVER`:
//!   `host=… port=… database=… user=… password=…`
//! - `ONTOLOGY_TEST_ODBC`: a connection string to the PostgreSQL above.

#![cfg(any(
    feature = "postgres",
    feature = "sqlserver",
    feature = "mysql",
    feature = "odbc"
))]

use std::collections::HashMap;

use ontology_io::connect::{ConnectError, Connector, Query};
use serde_json::{json, Value};

fn raw(var: &str) -> Option<String> {
    match std::env::var(var) {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => {
            assert!(
                std::env::var("ONTOLOGY_TEST_ENGINES").as_deref() != Ok("required"),
                "{var} is not set"
            );
            eprintln!("{var} is not set: skipped");
            None
        }
    }
}

#[allow(dead_code)] // not with `odbc` alone
fn engine(var: &str) -> Option<HashMap<String, String>> {
    Some(
        raw(var)?
            .split_whitespace()
            .filter_map(|p| p.split_once('='))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
}

/// A cell as an integer, whether the driver typed it or rendered it.
fn num(v: &Value) -> i64 {
    v.as_i64()
        .or_else(|| v.as_str()?.trim().parse().ok())
        .unwrap_or_else(|| panic!("not an integer: {v}"))
}

/// What every connector must read from the seeded table, the same way.
async fn reads_the_seeded_table(c: &dyn Connector, table: &str) {
    c.ping().await.expect("ping");
    let tables = c.tables().await.expect("tables");
    assert!(
        tables
            .iter()
            .any(|t| t.qualified().eq_ignore_ascii_case(table)),
        "{table} among {tables:?}"
    );
    let columns = c.columns(table).await.expect("columns");
    assert_eq!(
        columns
            .iter()
            .map(|c| c.name.to_lowercase())
            .collect::<Vec<_>>(),
        ["id", "name", "price", "ok", "updated", "note"]
    );

    // A cap: the first rows, and the result says it was cut.
    let cut = c
        .query(&Query {
            from: table.into(),
            after: None,
            max_rows: 2,
        })
        .await
        .expect("capped");
    assert_eq!(cut.rows.len(), 2);
    assert!(cut.truncated);

    // A watermark: the rows past it, in its order.
    let all = c
        .query(&Query {
            from: table.into(),
            after: Some(("id".into(), json!(0))),
            max_rows: 10,
        })
        .await
        .expect("all rows");
    assert!(!all.truncated);
    let col = |name: &str| all.column_index(name).expect(name);
    assert_eq!(
        all.rows
            .iter()
            .map(|r| num(&r[col("id")]))
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    let past = c
        .query(&Query {
            from: table.into(),
            after: Some(("id".into(), all.rows[0][col("id")].clone())),
            max_rows: 10,
        })
        .await
        .expect("past the watermark");
    assert_eq!(
        past.rows.iter().map(|r| num(&r[0])).collect::<Vec<_>>(),
        [2, 3],
        "the value a sync kept is the value the next one binds"
    );
    // A date the engine gave is a watermark the engine takes back.
    let by_date = c
        .query(&Query {
            from: table.into(),
            after: Some(("updated".into(), all.rows[1][col("updated")].clone())),
            max_rows: 10,
        })
        .await
        .expect("past a date");
    assert_eq!(
        by_date.rows.iter().map(|r| num(&r[0])).collect::<Vec<_>>(),
        [3]
    );

    // Text as written, a null as a null, a decimal with its digits.
    assert_eq!(all.rows[1][col("name")], "O'Brien & fils — été");
    assert!(all.rows[1][col("note")].is_null());
    assert_eq!(all.rows[0][col("note")], "first");
    let price = &all.rows[0][col("price")];
    assert_eq!(
        price
            .as_f64()
            .or_else(|| price.as_str()?.parse().ok())
            .expect("a price"),
        120.5
    );

    // A statement, as a named query sends it.
    let statement = c
        .query(&Query {
            from: format!("select id, name from {table} where name = 'Valve'"),
            after: None,
            max_rows: 5,
        })
        .await
        .expect("statement");
    assert_eq!(statement.rows.len(), 1);
    assert_eq!(num(&statement.rows[0][0]), 3);
    assert!(!statement.truncated);

    // What the engine refuses is its answer, not a dead connection.
    let refused = c
        .query(&Query {
            from: "ontology_live_missing".into(),
            after: None,
            max_rows: 1,
        })
        .await
        .unwrap_err();
    assert!(matches!(refused, ConnectError::Query(_)), "{refused}");
}

/// What every connector that writes must do with the seeded rows `ids`
/// (three in a row) of `table`: commit within the guard, roll back past
/// it, and leave the engine's refusal as its answer. Safe to run again.
async fn writes_within_the_guard_only(c: &dyn Connector, table: &str, ids: [i64; 3]) {
    let (lo, hi) = (ids[0], ids[2]);
    let states = || async {
        c.query(&Query {
            from: format!("select id, state from {table} where id >= {lo} and id <= {hi}"),
            after: Some(("id".into(), json!(lo - 1))),
            max_rows: 10,
        })
        .await
        .expect("states")
        .rows
        .iter()
        .map(|r| r[1].as_str().unwrap_or("").trim().to_string())
        .collect::<Vec<_>>()
    };
    let all = format!("where id >= {lo} and id <= {hi}");
    assert_eq!(
        c.execute(&format!("update {table} set state = 'open' {all}"), 3)
            .await
            .expect("reset"),
        3
    );
    // Past the guard: nothing is changed.
    let over = c
        .execute(&format!("update {table} set state = 'lost' {all}"), 2)
        .await
        .unwrap_err();
    assert!(
        matches!(&over, ConnectError::Query(m) if m.contains("would change 3 rows, at most 2")),
        "{over}"
    );
    assert_eq!(states().await, ["open", "open", "open"]);
    // Within it: committed, and read back by a later connection.
    assert_eq!(
        c.execute(
            &format!("update {table} set state = 'done' where id = {}", ids[1]),
            1
        )
        .await
        .expect("one row"),
        1
    );
    assert_eq!(states().await, ["open", "done", "open"]);
    // No row is no error.
    assert_eq!(
        c.execute(&format!("delete from {table} where id = -1"), 1)
            .await
            .expect("no row"),
        0
    );
    let refused = c
        .execute("update ontology_live_missing set state = 'x'", 1)
        .await
        .unwrap_err();
    assert!(matches!(refused, ConnectError::Query(_)), "{refused}");
    assert_eq!(states().await, ["open", "done", "open"]);
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_is_read_as_seeded_and_only_read() {
    use ontology_io::connect::postgres::PostgresConnector;
    use std::time::{Duration, Instant};

    let Some(e) = engine("ONTOLOGY_TEST_POSTGRES") else {
        return;
    };
    let c = PostgresConnector {
        host: e["host"].clone(),
        addr: None,
        port: e["port"].parse().unwrap(),
        database: e["database"].clone(),
        user: e["user"].clone(),
        password: e["password"].clone(),
        encrypt: true,
        trust_cert: true,
        timeout_secs: 30,
    };
    let table = "public.ontology_live_item";
    reads_the_seeded_table(&c, table).await;

    // The engine's types arrive as JSON types.
    let rs = c
        .query(&Query {
            from: table.into(),
            after: Some(("id".into(), json!(2))),
            max_rows: 0,
        })
        .await
        .unwrap();
    let col = |name: &str| rs.column_index(name).unwrap();
    assert_eq!(rs.rows[0][col("id")], json!(3));
    assert_eq!(rs.rows[0][col("ok")], json!(true));
    assert_eq!(rs.rows[0][col("price")], json!("9.99"));
    assert_eq!(rs.columns[col("price")].data_type, "numeric");
    // No row still describes its columns.
    let none = c
        .query(&Query {
            from: format!("select id, name from {table} where id < 0"),
            after: None,
            max_rows: 5,
        })
        .await
        .unwrap();
    assert!(none.rows.is_empty());
    assert_eq!(none.columns.len(), 2);

    // A write is refused by the session, whatever the account may do.
    let write = c
        .query(&Query {
            from: "select nextval('ontology_live_seq')".into(),
            after: None,
            max_rows: 1,
        })
        .await
        .unwrap_err();
    assert!(
        matches!(&write, ConnectError::Query(m) if m.contains("read-only")),
        "{write}"
    );
    // One statement per call: a second one is not run.
    let two = c
        .query(&Query {
            from: "select 1; select nextval('ontology_live_seq')".into(),
            after: None,
            max_rows: 1,
        })
        .await
        .unwrap_err();
    assert!(matches!(two, ConnectError::Query(_)), "{two}");

    // But for an approved write, in its own transaction — one statement.
    writes_within_the_guard_only(&c, "public.ontology_live_write", [1, 2, 3]).await;
    let two = c
        .execute(
            "update public.ontology_live_write set state = 'a' where id = 1; delete from public.ontology_live_write",
            10,
        )
        .await
        .unwrap_err();
    assert!(matches!(two, ConnectError::Query(_)), "{two}");

    // A statement past the source's timeout is ended.
    let slow = PostgresConnector {
        timeout_secs: 1,
        ..c.clone()
    };
    let started = Instant::now();
    let late = slow
        .query(&Query {
            from: "select pg_sleep(20)".into(),
            after: None,
            max_rows: 1,
        })
        .await
        .unwrap_err();
    assert!(matches!(late, ConnectError::Query(_)), "{late}");
    assert!(started.elapsed() < Duration::from_secs(10));

    // The test engine's certificate is self-signed: refused unless the
    // source trusts it.
    let strict = PostgresConnector {
        trust_cert: false,
        ..c.clone()
    };
    let untrusted = strict.ping().await.unwrap_err();
    assert!(matches!(untrusted, ConnectError::Connect(_)), "{untrusted}");
    let wrong = PostgresConnector {
        password: "not-the-password".into(),
        ..c.clone()
    };
    let denied = wrong.ping().await.unwrap_err();
    assert!(matches!(denied, ConnectError::Connect(_)), "{denied}");
    // Without TLS when the source opts out.
    PostgresConnector {
        encrypt: false,
        ..c.clone()
    }
    .ping()
    .await
    .expect("plain");
    // The pinned address is where the connection goes, whatever the name.
    if let Ok(ip) = c.host.parse() {
        PostgresConnector {
            host: "db.invalid".into(),
            addr: Some(ip),
            ..c.clone()
        }
        .ping()
        .await
        .expect("pinned");
    }
}

/// `ONTOLOGY_TEST_MYSQL`: `host= port= database= user= password=`; the
/// engine's certificate may be self-signed (MariaDB makes itself one).
#[cfg(feature = "mysql")]
#[tokio::test]
async fn mysql_is_read_as_seeded_and_only_read() {
    use ontology_io::connect::mysql::MysqlConnector;
    use std::time::{Duration, Instant};

    let Some(e) = engine("ONTOLOGY_TEST_MYSQL") else {
        return;
    };
    let c = MysqlConnector {
        host: e["host"].clone(),
        addr: None,
        port: e["port"].parse().unwrap(),
        database: e["database"].clone(),
        user: e["user"].clone(),
        password: e["password"].clone(),
        encrypt: true,
        trust_cert: true,
        timeout_secs: 30,
    };
    let table = format!("{}.ontology_live_item", e["database"]);
    reads_the_seeded_table(&c, &table).await;

    // The engine's types arrive as JSON types; a tinyint(1) is its 0 or 1.
    let rs = c
        .query(&Query {
            from: table.clone(),
            after: Some(("id".into(), json!(2))),
            max_rows: 0,
        })
        .await
        .unwrap();
    let col = |name: &str| rs.column_index(name).unwrap();
    assert_eq!(rs.rows[0][col("id")], json!(3));
    assert_eq!(rs.rows[0][col("ok")], json!(1));
    assert_eq!(rs.rows[0][col("price")], json!("9.99"));
    assert_eq!(rs.rows[0][col("updated")], json!("2026-03-01 10:00:00"));
    assert_eq!(rs.columns[col("price")].data_type, "newdecimal");

    // A write is refused by the session, whatever the account may do.
    let write = c
        .query(&Query {
            from: "select ontology_live_touch()".into(),
            after: None,
            max_rows: 1,
        })
        .await
        .unwrap_err();
    assert!(
        matches!(&write, ConnectError::Query(m) if m.to_lowercase().contains("read only")),
        "{write}"
    );
    // One statement per call: a second one is not run.
    let two = c
        .query(&Query {
            from: "select 1; select 2".into(),
            after: None,
            max_rows: 1,
        })
        .await
        .unwrap_err();
    assert!(matches!(two, ConnectError::Query(_)), "{two}");

    // But for an approved write, in its own transaction.
    writes_within_the_guard_only(
        &c,
        &format!("{}.ontology_live_write", e["database"]),
        [1, 2, 3],
    )
    .await;

    // A statement past the source's timeout is ended.
    let slow = MysqlConnector {
        timeout_secs: 1,
        ..c.clone()
    };
    let started = Instant::now();
    let late = slow
        .query(&Query {
            from: "select sleep(20)".into(),
            after: None,
            max_rows: 1,
        })
        .await
        .unwrap_err();
    assert!(matches!(late, ConnectError::Query(_)), "{late}");
    assert!(started.elapsed() < Duration::from_secs(10));

    let wrong = MysqlConnector {
        password: "not-the-password".into(),
        ..c.clone()
    };
    let denied = wrong.ping().await.unwrap_err();
    assert!(matches!(denied, ConnectError::Connect(_)), "{denied}");
    // The test engine's certificate is self-signed: refused unless the
    // source trusts it.
    let strict = MysqlConnector {
        trust_cert: false,
        ..c.clone()
    };
    let untrusted = strict.ping().await.unwrap_err();
    assert!(matches!(untrusted, ConnectError::Connect(_)), "{untrusted}");
    // Without TLS when the source opts out.
    MysqlConnector {
        encrypt: false,
        ..c.clone()
    }
    .ping()
    .await
    .expect("plain");
    // The pinned address is where the connection goes, whatever the name.
    if let Ok(ip) = c.host.parse() {
        MysqlConnector {
            host: "db.invalid".into(),
            addr: Some(ip),
            ..c.clone()
        }
        .ping()
        .await
        .expect("pinned");
    }
}

#[cfg(feature = "sqlserver")]
#[tokio::test]
async fn sql_server_is_read_as_seeded() {
    use ontology_io::connect::sqlserver::SqlServerConnector;

    let Some(e) = engine("ONTOLOGY_TEST_SQLSERVER") else {
        return;
    };
    let c = SqlServerConnector {
        host: e["host"].clone(),
        addr: None,
        port: e["port"].parse().unwrap(),
        database: e["database"].clone(),
        user: e["user"].clone(),
        password: e["password"].clone(),
        encrypt: true,
        trust_cert: true,
        timeout_secs: 30,
    };
    let table = "dbo.ontology_live_item";
    reads_the_seeded_table(&c, table).await;

    let rs = c
        .query(&Query {
            from: table.into(),
            after: Some(("id".into(), json!(2))),
            max_rows: 0,
        })
        .await
        .unwrap();
    let col = |name: &str| rs.column_index(name).unwrap();
    assert_eq!(rs.rows[0][col("id")], json!(3));
    assert_eq!(rs.rows[0][col("ok")], json!(true));
    assert_eq!(rs.rows[0][col("updated")], json!("2026-03-01T10:00:00"));
    writes_within_the_guard_only(&c, "dbo.ontology_live_write", [1, 2, 3]).await;

    let strict = SqlServerConnector {
        trust_cert: false,
        ..c.clone()
    };
    let untrusted = strict.ping().await.unwrap_err();
    assert!(matches!(untrusted, ConnectError::Connect(_)), "{untrusted}");
    let wrong = SqlServerConnector {
        password: "not-the-password".into(),
        ..c.clone()
    };
    let denied = wrong.ping().await.unwrap_err();
    assert!(matches!(denied, ConnectError::Connect(_)), "{denied}");
}

#[cfg(feature = "odbc")]
#[tokio::test]
async fn an_odbc_driver_reads_the_seeded_table() {
    use ontology_io::connect::odbc::OdbcConnector;

    let Some(connection_string) = raw("ONTOLOGY_TEST_ODBC") else {
        return;
    };
    let c = OdbcConnector {
        connection_string,
        timeout_secs: 30,
    };
    reads_the_seeded_table(&c, "public.ontology_live_item").await;
    // Its own rows: the PostgreSQL test writes the first three.
    writes_within_the_guard_only(&c, "public.ontology_live_write", [11, 12, 13]).await;

    let nowhere = OdbcConnector {
        connection_string: "Driver={No Such Driver};Server=127.0.0.1;".into(),
        timeout_secs: 5,
    };
    let absent = nowhere.ping().await.unwrap_err();
    assert!(matches!(absent, ConnectError::Connect(_)), "{absent}");

    // The driver that just read the table is among those listed.
    let (drivers, _dsns) = ontology_io::connect::odbc::catalog().unwrap();
    assert!(!drivers.is_empty());
}
