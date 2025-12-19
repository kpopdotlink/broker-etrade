# broker-etrade

E*TRADE (Morgan Stanley) OpenAPI 연동 플러그인 for KL Investment.

## 개요

E*TRADE API를 통해 미국 주식, ETF, 옵션 거래를 지원하는 WASM 플러그인입니다.

### 지원 자산

| 자산군 | 구현 상태 | 설명 |
|--------|----------|------|
| 미국 주식 | ✅ 완료 | NYSE, NASDAQ, AMEX |
| ETF | ✅ 완료 | 모든 미국 상장 ETF |
| 옵션 | 🚧 계획 | 향후 지원 예정 |

### 플러그인 인터페이스

| 함수 | 연동 API | 상태 |
|------|----------|------|
| `initialize()` | OAuth 1.0a 인증 | ✅ |
| `get_accounts()` | /v1/accounts/list | ✅ |
| `get_positions()` | /v1/accounts/{id}/portfolio | ✅ |
| `submit_order()` | /v1/accounts/{id}/orders/place | ✅ |

## Persona 연동

KL Investment v0.8.9부터 **Persona(가상 서브계좌)** 기능을 지원합니다.

```json
// RPC: personas.create
{
  "name": "US Growth Portfolio",
  "broker_id": "broker-etrade",
  "broker_account_id": "your-account-id",
  "budget": 50000
}
```

## 설정

### 1. E*TRADE Developer 앱 등록

1. [E*TRADE Developer](https://developer.etrade.com)에서 앱 등록
2. Sandbox 또는 Production API Key 발급

### 2. 브로커 초기화

```json
// RPC: plugins.initializeBroker
{
  "plugin_id": "broker-etrade",
  "credentials": {
    "consumer_key": "your-consumer-key",
    "consumer_secret": "your-consumer-secret"
  }
}
```

### 3. OAuth 인증 완료

E*TRADE는 OAuth 1.0a를 사용합니다. 첫 연결 시:
1. Request Token 획득
2. 사용자 브라우저에서 인증
3. Access Token 교환

### 4. 빌드

```bash
# WASM 타겟 추가 (최초 1회)
rustup target add wasm32-wasip1

# 빌드
cargo build --target wasm32-wasip1 --release

# 결과물: target/wasm32-wasip1/release/broker_etrade.wasm
```

## 아키텍처

```
broker-etrade/
├── src/
│   ├── lib.rs          # WASM 진입점, 플러그인 인터페이스
│   ├── http.rs         # HTTP 호스트 함수 래퍼
│   └── etrade.rs       # E*TRADE API 클라이언트
├── manifest.json       # 플러그인 매니페스트
├── Cargo.toml
└── README.md
```

## API 환경

| 환경 | Base URL |
|------|----------|
| Production | `https://api.etrade.com` |
| Sandbox | `https://apisb.etrade.com` |

## 데이터 매핑

### 잔고 조회 (get_accounts)

```
E*TRADE API                    → Plugin API
────────────────────────────────────────────────
totalAccountValue              → AccountBalance.total_equity
netMv                          → AccountBalance.available_cash
totalLongValue                 → AccountBalance.buying_power
```

### 주문 (submit_order)

```
Plugin API                     → E*TRADE API
────────────────────────────────────────────────
OrderType::Market              → priceType = "MARKET"
OrderType::Limit               → priceType = "LIMIT"
OrderSide::Buy                 → orderAction = "BUY"
OrderSide::Sell                → orderAction = "SELL"
```

## 제한사항

1. **OAuth 1.0a**: 복잡한 인증 흐름, 브라우저 인증 필요
2. **허용된 호스트만**: `api.etrade.com`, `apisb.etrade.com`만 접근 가능
3. **API Rate Limit**: E*TRADE API 호출 제한 준수 필요

## 참고 자료

- [E*TRADE Developer Portal](https://developer.etrade.com)
- [E*TRADE API Documentation](https://apisb.etrade.com/docs/api/account/api-account-v1.html)
- [KL Investment 메인 프로젝트](https://github.com/kpopdotlink/klinvestment)
