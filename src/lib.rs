//! E*TRADE (Morgan Stanley) Broker Plugin for KL Investment
//!
//! This plugin integrates with E*TRADE API to provide:
//! - Account balance and positions
//! - Order submission (stocks, ETFs, options)
//!
//! ## Authentication
//! E*TRADE uses OAuth 1.0a for authentication.
//!
//! ## API Environments
//! - Production: https://api.etrade.com
//! - Sandbox: https://apisb.etrade.com

mod http;
mod etrade;

use chrono::Utc;
use std::collections::HashMap;
use std::slice;
use std::sync::Mutex;

use etrade::ETradeClient;
use models::order::{Order, OrderStatus};
use models::portfolio::{AccountBalance, AccountSummary, Position};
use plugin_api::{
    GetAccountsRequest, GetAccountsResponse, GetPositionsRequest, GetPositionsResponse,
    SubmitOrderRequest, SubmitOrderResponse,
};

// --- State Management ---

struct BrokerState {
    client: Option<ETradeClient>,
    orders: HashMap<String, Order>,
    next_order_id: u64,
}

impl BrokerState {
    fn new() -> Self {
        Self {
            client: None,
            orders: HashMap::new(),
            next_order_id: 1,
        }
    }
}

lazy_static::lazy_static! {
    static ref STATE: Mutex<BrokerState> = Mutex::new(BrokerState::new());
}

// --- WASM Exports ---

/// Memory allocation for host communication
#[no_mangle]
pub extern "C" fn alloc(len: i32) -> i32 {
    let mut buf: Vec<u8> = Vec::with_capacity(len as usize);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr as usize as i32
}

/// Initialize plugin with configuration
#[no_mangle]
pub extern "C" fn initialize(ptr: i32, len: i32) -> u64 {
    let config_json: serde_json::Value = parse_request(ptr, len);

    let mut state = STATE.lock().unwrap();

    // Parse configuration from secrets
    let consumer_key = config_json
        .get("consumer_key")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let consumer_secret = config_json
        .get("consumer_secret")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let oauth_token = config_json
        .get("oauth_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let oauth_token_secret = config_json
        .get("oauth_token_secret")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let is_sandbox = config_json
        .get("is_sandbox")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // Validate configuration
    if consumer_key.is_empty() || consumer_secret.is_empty() {
        return serialize_response(&serde_json::json!({
            "success": false,
            "error": "Missing required configuration: consumer_key or consumer_secret"
        }));
    }

    // Check if OAuth tokens are available
    let has_tokens = oauth_token.is_some() && oauth_token_secret.is_some();

    if has_tokens {
        // Create E*TRADE client with full credentials
        let client = ETradeClient::new(
            consumer_key,
            consumer_secret,
            oauth_token.unwrap(),
            oauth_token_secret.unwrap(),
            is_sandbox,
        );
        state.client = Some(client);

        serialize_response(&serde_json::json!({
            "success": true,
            "message": format!("E*TRADE plugin initialized ({})", if is_sandbox { "sandbox" } else { "production" })
        }))
    } else {
        serialize_response(&serde_json::json!({
            "success": true,
            "message": "E*TRADE plugin initialized. OAuth authorization required.",
            "requires_auth": true,
            "auth_url": "https://us.etrade.com/e/t/etws/authorize"
        }))
    }
}

/// Get available accounts
#[no_mangle]
pub extern "C" fn get_accounts(ptr: i32, len: i32) -> u64 {
    let _req: GetAccountsRequest = parse_request(ptr, len);

    let state = STATE.lock().unwrap();

    let client = match state.client.as_ref() {
        Some(c) => c,
        None => {
            return serialize_response(&GetAccountsResponse {
                accounts: vec![create_error_account("Plugin not initialized or OAuth not completed")],
            });
        }
    };

    match client.list_accounts() {
        Ok(accounts) => {
            let response = GetAccountsResponse { accounts };
            serialize_response(&response)
        }
        Err(e) => {
            eprintln!("[broker-etrade] Failed to fetch accounts: {}", e);
            serialize_response(&GetAccountsResponse {
                accounts: vec![create_error_account(&e)],
            })
        }
    }
}

/// Get positions for an account
#[no_mangle]
pub extern "C" fn get_positions(ptr: i32, len: i32) -> u64 {
    let req: GetPositionsRequest = parse_request(ptr, len);

    let state = STATE.lock().unwrap();

    let client = match state.client.as_ref() {
        Some(c) => c,
        None => {
            return serialize_response(&GetPositionsResponse { positions: vec![] });
        }
    };

    match client.get_positions(&req.account_id) {
        Ok(positions) => {
            let response = GetPositionsResponse { positions };
            serialize_response(&response)
        }
        Err(e) => {
            eprintln!("[broker-etrade] Failed to fetch positions: {}", e);
            serialize_response(&GetPositionsResponse { positions: vec![] })
        }
    }
}

/// Submit an order
#[no_mangle]
pub extern "C" fn submit_order(ptr: i32, len: i32) -> u64 {
    let req: SubmitOrderRequest = parse_request(ptr, len);
    let mut state = STATE.lock().unwrap();

    let client = match state.client.as_ref() {
        Some(c) => c,
        None => {
            return serialize_response(&SubmitOrderResponse {
                order: create_error_order(&req, "Plugin not initialized"),
            });
        }
    };

    match client.submit_order(&req.account_id, &req.order) {
        Ok(mut order) => {
            // Track order locally
            let order_id = order.id.clone();
            if order.persona_id.is_empty() {
                order.persona_id = req.order.persona_id.clone();
            }
            state.orders.insert(order_id, order.clone());
            state.next_order_id += 1;

            serialize_response(&SubmitOrderResponse { order })
        }
        Err(e) => {
            eprintln!("[broker-etrade] Order failed: {}", e);
            serialize_response(&SubmitOrderResponse {
                order: create_error_order(&req, &e),
            })
        }
    }
}

// --- Helper Functions ---

fn parse_request<T: serde::de::DeserializeOwned>(ptr: i32, len: i32) -> T {
    let slice = unsafe { slice::from_raw_parts(ptr as *const u8, len as usize) };
    serde_json::from_slice(slice).expect("Failed to parse request")
}

fn serialize_response<T: serde::Serialize>(response: &T) -> u64 {
    let res_bytes = serde_json::to_vec(response).expect("Failed to serialize response");

    let out_len = res_bytes.len() as i32;
    let out_ptr = alloc(out_len);

    unsafe {
        std::ptr::copy_nonoverlapping(res_bytes.as_ptr(), out_ptr as *mut u8, out_len as usize);
    }

    ((out_ptr as u64) << 32) | (out_len as u64)
}

fn create_error_account(error: &str) -> AccountSummary {
    AccountSummary {
        id: "error".to_string(),
        name: format!("Error: {}", error),
        broker_id: "broker-etrade".to_string(),
        is_paper: true,
        balance: AccountBalance {
            currency: "USD".to_string(),
            total_equity: 0.0,
            available_cash: 0.0,
            buying_power: 0.0,
            locked_cash: 0.0,
        },
        positions: vec![],
        updated_at: Utc::now(),
        extensions: None,
    }
}

fn create_error_order(req: &SubmitOrderRequest, error: &str) -> Order {
    Order {
        id: format!("error_{}", Utc::now().timestamp_millis()),
        request: req.order.clone(),
        status: OrderStatus::Rejected,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        average_filled_price: None,
        filled_quantity: 0.0,
        extensions: Some({
            let mut map = HashMap::new();
            map.insert("error".to_string(), serde_json::Value::String(error.to_string()));
            map
        }),
        persona_id: req.order.persona_id.clone(),
    }
}
