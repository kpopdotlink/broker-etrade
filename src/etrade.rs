//! E*TRADE API Client
//!
//! Implements OAuth 1.0a authentication and E*TRADE API endpoints.

use crate::http::{HttpMethod, HttpRequest, execute};
use chrono::Utc;
use models::order::{Order, OrderRequest, OrderSide, OrderStatus, OrderType};
use models::portfolio::{AccountBalance, AccountSummary, Position};
use serde::Deserialize;
use std::collections::HashMap;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use hmac::{Hmac, Mac};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

const PRODUCTION_URL: &str = "https://api.etrade.com";
const SANDBOX_URL: &str = "https://apisb.etrade.com";

pub struct ETradeClient {
    consumer_key: String,
    consumer_secret: String,
    oauth_token: String,
    oauth_token_secret: String,
    base_url: String,
    is_sandbox: bool,
}

impl ETradeClient {
    pub fn new(
        consumer_key: String,
        consumer_secret: String,
        oauth_token: String,
        oauth_token_secret: String,
        is_sandbox: bool,
    ) -> Self {
        Self {
            consumer_key,
            consumer_secret,
            oauth_token,
            oauth_token_secret,
            base_url: if is_sandbox { SANDBOX_URL } else { PRODUCTION_URL }.to_string(),
            is_sandbox,
        }
    }

    /// Generate OAuth 1.0a signature
    fn generate_oauth_signature(
        &self,
        method: &str,
        url: &str,
        params: &[(String, String)],
    ) -> String {
        let mut sorted_params = params.to_vec();
        sorted_params.sort_by(|a, b| a.0.cmp(&b.0));

        let param_string: String = sorted_params
            .iter()
            .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        let base_string = format!(
            "{}&{}&{}",
            method.to_uppercase(),
            percent_encode(url),
            percent_encode(&param_string)
        );

        let signing_key = format!(
            "{}&{}",
            percent_encode(&self.consumer_secret),
            percent_encode(&self.oauth_token_secret)
        );

        let mut mac = HmacSha1::new_from_slice(signing_key.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(base_string.as_bytes());
        let result = mac.finalize();

        BASE64.encode(result.into_bytes())
    }

    /// Build OAuth Authorization header
    fn build_auth_header(&self, method: &str, url: &str) -> String {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string();

        let nonce = format!("{:016x}", rand::random::<u64>());

        let mut oauth_params = vec![
            ("oauth_consumer_key".to_string(), self.consumer_key.clone()),
            ("oauth_token".to_string(), self.oauth_token.clone()),
            ("oauth_signature_method".to_string(), "HMAC-SHA1".to_string()),
            ("oauth_timestamp".to_string(), timestamp),
            ("oauth_nonce".to_string(), nonce),
            ("oauth_version".to_string(), "1.0".to_string()),
        ];

        let signature = self.generate_oauth_signature(method, url, &oauth_params);
        oauth_params.push(("oauth_signature".to_string(), signature));

        let header_value: String = oauth_params
            .iter()
            .map(|(k, v)| format!("{}=\"{}\"", k, percent_encode(v)))
            .collect::<Vec<_>>()
            .join(", ");

        format!("OAuth {}", header_value)
    }

    fn api_get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let url = format!("{}{}", self.base_url, path);
        let auth_header = self.build_auth_header("GET", &url);

        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), auth_header);
        headers.insert("Accept".to_string(), "application/json".to_string());

        let response = execute(HttpRequest {
            method: HttpMethod::Get,
            url,
            headers,
            body: None,
            timeout_ms: 30000,
        });

        if !response.is_success() {
            return Err(format!(
                "API error {}: {}",
                response.status,
                response.error.unwrap_or(response.body)
            ));
        }

        response.json::<T>()
    }

    fn api_post<T: serde::de::DeserializeOwned, B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T, String> {
        let url = format!("{}{}", self.base_url, path);
        let auth_header = self.build_auth_header("POST", &url);

        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), auth_header);
        headers.insert("Accept".to_string(), "application/json".to_string());
        headers.insert("Content-Type".to_string(), "application/json".to_string());

        let body_str = serde_json::to_string(body)
            .map_err(|e| e.to_string())?;

        let response = execute(HttpRequest {
            method: HttpMethod::Post,
            url,
            headers,
            body: Some(body_str),
            timeout_ms: 30000,
        });

        if !response.is_success() {
            return Err(format!(
                "API error {}: {}",
                response.status,
                response.error.unwrap_or(response.body)
            ));
        }

        response.json::<T>()
    }

    /// List all accounts
    pub fn list_accounts(&self) -> Result<Vec<AccountSummary>, String> {
        #[derive(Deserialize)]
        struct AccountListResponse {
            #[serde(rename = "AccountListResponse")]
            response: AccountListInner,
        }

        #[derive(Deserialize)]
        struct AccountListInner {
            #[serde(rename = "Accounts")]
            accounts: AccountsWrapper,
        }

        #[derive(Deserialize)]
        struct AccountsWrapper {
            #[serde(rename = "Account")]
            account: Vec<ETradeAccount>,
        }

        #[derive(Deserialize)]
        struct ETradeAccount {
            #[serde(rename = "accountId")]
            account_id: String,
            #[serde(rename = "accountIdKey")]
            account_id_key: String,
            #[serde(rename = "accountName")]
            account_name: Option<String>,
        }

        let resp: AccountListResponse = self.api_get("/v1/accounts/list")?;

        let mut accounts = Vec::new();
        for acct in resp.response.accounts.account {
            let (balance, positions) = self.get_account_details(&acct.account_id_key)?;

            accounts.push(AccountSummary {
                id: acct.account_id.clone(),
                name: acct.account_name.unwrap_or_else(|| format!("E*TRADE {}", acct.account_id)),
                broker_id: "broker-etrade".to_string(),
                is_paper: self.is_sandbox,
                balance,
                positions,
                updated_at: Utc::now(),
                extensions: Some({
                    let mut map = HashMap::new();
                    map.insert("account_id_key".to_string(),
                        serde_json::Value::String(acct.account_id_key));
                    map
                }),
            });
        }

        Ok(accounts)
    }

    fn get_account_details(&self, account_id_key: &str) -> Result<(AccountBalance, Vec<Position>), String> {
        #[derive(Deserialize)]
        struct BalanceResponse {
            #[serde(rename = "BalanceResponse")]
            response: BalanceInner,
        }

        #[derive(Deserialize)]
        struct BalanceInner {
            #[serde(rename = "Computed")]
            computed: Option<ComputedBalance>,
        }

        #[derive(Deserialize)]
        struct ComputedBalance {
            #[serde(rename = "RealTimeValues")]
            real_time: Option<RealTimeValues>,
        }

        #[derive(Deserialize)]
        struct RealTimeValues {
            #[serde(rename = "totalAccountValue")]
            total_account_value: Option<f64>,
            #[serde(rename = "netMv")]
            net_mv: Option<f64>,
            #[serde(rename = "totalLongValue")]
            total_long_value: Option<f64>,
        }

        let path = format!("/v1/accounts/{}/balance?instType=BROKERAGE&realTimeNAV=true", account_id_key);

        let balance = match self.api_get::<BalanceResponse>(&path) {
            Ok(resp) => {
                let rt = resp.response.computed
                    .and_then(|c| c.real_time);

                AccountBalance {
                    currency: "USD".to_string(),
                    total_equity: rt.as_ref().and_then(|r| r.total_account_value).unwrap_or(0.0),
                    available_cash: rt.as_ref().and_then(|r| r.net_mv).unwrap_or(0.0),
                    buying_power: rt.as_ref().and_then(|r| r.total_long_value).unwrap_or(0.0),
                    locked_cash: 0.0,
                }
            }
            Err(_) => AccountBalance {
                currency: "USD".to_string(),
                total_equity: 0.0,
                available_cash: 0.0,
                buying_power: 0.0,
                locked_cash: 0.0,
            }
        };

        // Get positions
        let positions = self.get_positions(account_id_key).unwrap_or_default();

        Ok((balance, positions))
    }

    /// Get positions for an account
    pub fn get_positions(&self, account_id: &str) -> Result<Vec<Position>, String> {
        #[derive(Deserialize)]
        struct PortfolioResponse {
            #[serde(rename = "PortfolioResponse")]
            response: Option<PortfolioInner>,
        }

        #[derive(Deserialize)]
        struct PortfolioInner {
            #[serde(rename = "AccountPortfolio")]
            account_portfolio: Option<Vec<AccountPortfolio>>,
        }

        #[derive(Deserialize)]
        struct AccountPortfolio {
            #[serde(rename = "Position")]
            position: Option<Vec<ETradePosition>>,
        }

        #[derive(Deserialize)]
        struct ETradePosition {
            #[serde(rename = "Product")]
            product: ProductInfo,
            quantity: Option<f64>,
            #[serde(rename = "costPerShare")]
            cost_per_share: Option<f64>,
            #[serde(rename = "marketValue")]
            market_value: Option<f64>,
            #[serde(rename = "totalGain")]
            total_gain: Option<f64>,
            #[serde(rename = "totalGainPct")]
            total_gain_pct: Option<f64>,
            #[serde(rename = "Quick")]
            quick: Option<QuickView>,
        }

        #[derive(Deserialize)]
        struct ProductInfo {
            symbol: String,
        }

        #[derive(Deserialize)]
        struct QuickView {
            #[serde(rename = "lastTrade")]
            last_trade: Option<f64>,
        }

        let path = format!("/v1/accounts/{}/portfolio", account_id);
        let resp: PortfolioResponse = self.api_get(&path)?;

        let mut positions = Vec::new();

        if let Some(inner) = resp.response {
            if let Some(portfolios) = inner.account_portfolio {
                for portfolio in portfolios {
                    if let Some(pos_list) = portfolio.position {
                        for pos in pos_list {
                            let current_price = pos.quick
                                .and_then(|q| q.last_trade)
                                .or(pos.cost_per_share)
                                .unwrap_or(0.0);

                            positions.push(Position {
                                symbol_id: pos.product.symbol,
                                quantity: pos.quantity.unwrap_or(0.0),
                                average_price: pos.cost_per_share.unwrap_or(0.0),
                                current_price,
                                unrealized_pnl: pos.total_gain.unwrap_or(0.0),
                                unrealized_pnl_percent: pos.total_gain_pct.unwrap_or(0.0),
                            });
                        }
                    }
                }
            }
        }

        Ok(positions)
    }

    /// Submit an order
    pub fn submit_order(&self, account_id: &str, order: &OrderRequest) -> Result<Order, String> {
        #[derive(serde::Serialize)]
        struct PlaceOrderRequest {
            #[serde(rename = "PlaceOrderRequest")]
            request: PlaceOrderInner,
        }

        #[derive(serde::Serialize)]
        struct PlaceOrderInner {
            #[serde(rename = "orderType")]
            order_type: String,
            #[serde(rename = "clientOrderId")]
            client_order_id: String,
            #[serde(rename = "Order")]
            order: Vec<OrderInner>,
        }

        #[derive(serde::Serialize)]
        struct OrderInner {
            #[serde(rename = "allOrNone")]
            all_or_none: bool,
            #[serde(rename = "priceType")]
            price_type: String,
            #[serde(rename = "orderTerm")]
            order_term: String,
            #[serde(rename = "marketSession")]
            market_session: String,
            #[serde(rename = "limitPrice")]
            #[serde(skip_serializing_if = "Option::is_none")]
            limit_price: Option<f64>,
            #[serde(rename = "Instrument")]
            instrument: Vec<InstrumentInner>,
        }

        #[derive(serde::Serialize)]
        struct InstrumentInner {
            #[serde(rename = "Product")]
            product: ProductInner,
            #[serde(rename = "orderAction")]
            order_action: String,
            #[serde(rename = "quantityType")]
            quantity_type: String,
            quantity: f64,
        }

        #[derive(serde::Serialize)]
        struct ProductInner {
            #[serde(rename = "securityType")]
            security_type: String,
            symbol: String,
        }

        let price_type = match order.order_type {
            OrderType::Market => "MARKET",
            OrderType::Limit => "LIMIT",
            OrderType::Stop => "STOP",
            OrderType::StopLimit => "STOP_LIMIT",
        };

        let order_action = match order.side {
            OrderSide::Buy => "BUY",
            OrderSide::Sell => "SELL",
        };

        let client_order_id = format!("KL{:016x}", rand::random::<u64>());

        let req = PlaceOrderRequest {
            request: PlaceOrderInner {
                order_type: "EQ".to_string(),
                client_order_id: client_order_id.clone(),
                order: vec![OrderInner {
                    all_or_none: false,
                    price_type: price_type.to_string(),
                    order_term: "GOOD_FOR_DAY".to_string(),
                    market_session: "REGULAR".to_string(),
                    limit_price: order.limit_price,
                    instrument: vec![InstrumentInner {
                        product: ProductInner {
                            security_type: "EQ".to_string(),
                            symbol: order.symbol_id.clone(),
                        },
                        order_action: order_action.to_string(),
                        quantity_type: "QUANTITY".to_string(),
                        quantity: order.quantity,
                    }],
                }],
            },
        };

        #[derive(Deserialize)]
        struct PlaceOrderResponse {
            #[serde(rename = "PlaceOrderResponse")]
            response: PlaceOrderResult,
        }

        #[derive(Deserialize)]
        struct PlaceOrderResult {
            #[serde(rename = "OrderIds")]
            order_ids: Option<Vec<OrderIdInfo>>,
        }

        #[derive(Deserialize)]
        struct OrderIdInfo {
            #[serde(rename = "orderId")]
            order_id: i64,
        }

        let path = format!("/v1/accounts/{}/orders/place", account_id);
        let resp: PlaceOrderResponse = self.api_post(&path, &req)?;

        let order_id = resp.response.order_ids
            .and_then(|ids| ids.first().map(|o| o.order_id.to_string()))
            .unwrap_or_else(|| client_order_id.clone());

        Ok(Order {
            id: order_id,
            request: order.clone(),
            status: OrderStatus::Submitted,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            filled_quantity: 0.0,
            average_filled_price: None,
            extensions: Some({
                let mut map = HashMap::new();
                map.insert("client_order_id".to_string(),
                    serde_json::Value::String(client_order_id));
                map
            }),
            persona_id: order.persona_id.clone(),
        })
    }
}

/// URL percent encoding (RFC 3986)
fn percent_encode(s: &str) -> String {
    let mut result = String::new();
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}
