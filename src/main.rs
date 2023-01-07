use axum::{
    extract::{Path, Query},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use nipper::Document;
use reqwest::StatusCode;
use serde::{
    de::{self},
    Deserialize, Deserializer, Serialize,
};
use std::{collections::HashMap, env, fmt, net::SocketAddr, str::FromStr};

// Use Jemalloc only for musl-64 bits platforms
#[cfg(all(target_env = "musl", target_pointer_width = "64"))]
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

#[tokio::main]
async fn main() {
    let _ = match env::var("url") {
        Ok(val) => val,
        Err(_) => panic!("url not set"),
    };
    let app = Router::new()
        .route("/api/query_result", get(query))
        .route("/api/account/:account", get(account));
    let addr = SocketAddr::from(([0, 0, 0, 0], 80));
    println!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn account(
    Path(account): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<SuccessResult, ErrorResult> {
    let query_result = query_account(account).await;
    match query_result {
        Ok(result) => return Ok(SuccessResult::new(get_account_data(result, params))),
        Err(e) => Err(ErrorResult::new(e.to_string())),
    }
}

async fn query(Query(params): Query<Params>) -> Result<SuccessResult, ErrorResult> {
    let query_result = query_table(params).await;
    match query_result {
        Ok(result) => {
            if result.success {
                let data = result.data;
                if let Some(data) = data {
                    let table_str = data.table;
                    return Ok(SuccessResult::new(get_table_data(table_str)));
                }
                Ok(SuccessResult::default())
            } else {
                Err(ErrorResult::new(
                    result.error.unwrap_or("Something went wrong".into()),
                ))
            }
        }
        Err(e) => Err(ErrorResult::new(e.to_string())),
    }
}

async fn query_table(params: Params) -> Result<QueryResult, reqwest::Error> {
    let url = env::var("url").unwrap_or_default();

    let query_url = format!(
        "{}/api/query_result?query_string={}",
        url, params.query_string
    );
    // 先请求页面以刷新数据
    let _ = reqwest::get(format!("{}/income_statement/", url))
        .await?
        .text()
        .await?;
    let result = reqwest::get(query_url).await?.json::<QueryResult>().await?;

    Ok(result)
}

async fn query_account(account: String) -> Result<String, reqwest::Error> {
    let url = env::var("url").unwrap_or_default();
    let _ = reqwest::get(format!("{}/income_statement/", url))
        .await?
        .text()
        .await?;
    let url = format!("{}/account/{}", url, account);
    // 先请求页面以刷新数据
    let result = reqwest::get(url).await?.text().await?;
    Ok(result)
}

fn get_table_data(table_str: String) -> Vec<HashMap<String, String>> {
    let document = Document::from(table_str.as_str());
    let table_title = document.select("thead").select("tr").select("th");
    let table_lines = document.select("tbody").select("tr");
    let mut titles = Vec::new();
    table_title.iter().for_each(|node| {
        titles.push(node.text().to_string());
    });

    let mut result: Vec<HashMap<String, String>> = Vec::new();
    table_lines.iter().for_each(|node| {
        let mut line: HashMap<String, String> = HashMap::new();

        for (i, el) in node.select("td").iter().enumerate() {
            let title = titles.get(i).unwrap();
            let value = el.text().trim().to_string();
            line.insert(title.to_string(), value);
        }
        result.push(line);
    });
    result
}

fn get_account_data(html: String, params: AccountParams) -> Vec<HashMap<String, String>> {
    let document = Document::from(html.as_str());
    let table = document.select(".flex-table");
    let data_lines = table.select(".transaction");
    let mut result: Vec<HashMap<String, String>> = Vec::new();
    data_lines.iter().for_each(|line| {
        let mut result_item: HashMap<String, String> = HashMap::new();
        let date = line.select(".datecell").text().to_string();
        if result.iter().any(|item| item.get("date") == Some(&date)) {
            return;
        }
        let mut changed: f32 = line
            .select(".change")
            .text()
            .replace("CNY", "")
            .trim()
            .parse()
            .unwrap();
        let mut balance: f32 = line
            .select("span:nth-child(6)")
            .text()
            .replace("CNY", "")
            .trim()
            .parse()
            .unwrap();

        if Some(true) == params.negate {
            changed = 0.0 - changed;
            balance = 0.0 - balance;
        }
        result_item.insert("date".into(), date);
        result_item.insert("changed".into(), changed.to_string());
        result_item.insert("balance".into(), balance.to_string());
        result.push(result_item);
    });
    result.reverse();
    result
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Params {
    query_string: String,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    account: Option<String>,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    filter: Option<String>,
    #[serde(default, deserialize_with = "empty_string_as_none")]
    time: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AccountParams {
    #[serde(default, deserialize_with = "empty_string_as_none")]
    negate: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct QueryResult {
    error: Option<String>,
    success: bool,
    data: Option<QueryResultData>,
}

#[derive(Debug, Serialize, Deserialize)]
struct QueryResultData {
    table: String,
}

/// Serde deserialization decorator to map empty Strings to None,
fn empty_string_as_none<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: fmt::Display,
{
    let opt = Option::<String>::deserialize(de)?;
    match opt.as_deref() {
        None | Some("") => Ok(None),
        Some(s) => FromStr::from_str(s).map_err(de::Error::custom).map(Some),
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ErrorResult {
    error: String,
    success: bool,
}

impl ErrorResult {
    fn new(error: String) -> ErrorResult {
        ErrorResult {
            error: error,
            success: false,
        }
    }
}

impl IntoResponse for ErrorResult {
    fn into_response(self) -> Response {
        let body = Json(self);
        (StatusCode::OK, body).into_response()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SuccessResult {
    success: bool,
    data: Vec<HashMap<String, String>>,
}

impl SuccessResult {
    fn new(data: Vec<HashMap<String, String>>) -> SuccessResult {
        SuccessResult {
            success: true,
            data,
        }
    }

    fn default() -> SuccessResult {
        SuccessResult {
            success: true,
            data: Vec::new(),
        }
    }
}

impl IntoResponse for SuccessResult {
    fn into_response(self) -> Response {
        let body = Json(self);
        (StatusCode::OK, body).into_response()
    }
}
