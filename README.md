# Latency Budget

[한국어](#한국어) · [English](#english)

## 한국어

**Latency Budget**은 로보틱스·VLA 개발자를 위한 작은 정적 웹 앱입니다.
"이 인지 → 정책 → 구동 파이프라인이 제어 루프 데드라인을 맞출 수 있을까? 병목은 어느 단계일까?"라는 질문에 바로 답합니다.
Rust로 작성되어 WebAssembly로 브라우저에서만 실행됩니다.

### 기능
- **편집 가능한 파이프라인 예산** — 기본 6단계(센서 캡처, 디코드, 인지, 정책, 안전 필터, 구동). 각 단계의 이름, 일반 지연(ms), p95 지연(ms), 병렬 워커 수(1–8)를 수정하고, 단계를 추가·삭제·순서 변경할 수 있습니다.
- **타임라인 워터폴** — 단계별 비례 블록, 일반/p95 오버레이, 데드라인 마커, 병목 하이라이트, 툴팁, 상태 HUD.
- **예시 프리셋 3종** — 비전 제어, 로컬 VLA 정책, 원격 보조 정책. 모두 가상의 예시 값이며 자유롭게 수정할 수 있습니다. 특정 제품·벤더와 무관합니다.
- **CSV 내보내기** — 현재 예산을 로컬 파일로 다운로드합니다.
- 한국어 기본, 영어 전환 버튼.

### 모델
- 순차 지연 = 모든 단계 지연의 합 (일반, p95 각각).
- 파이프라인 정상 상태 처리율 = `1000 / max(typical / workers)` Hz. 이 값을 결정하는 단계가 **병목**이며(동률이면 앞 단계), 순차 일반 지연 중 그 단계의 비율을 함께 표시합니다.
- 마진 = 데드라인 − 합계 (일반, p95 각각).
- 판정 (경계값 포함):
  - **통과**: p95 합계 ≤ 데드라인 그리고 최대 처리율 ≥ 목표 Hz
  - **주의**: 일반 합계 ≤ 데드라인이지만 p95 합계 > 데드라인
  - **실패**: 일반 합계 > 데드라인 또는 최대 처리율 < 목표 Hz
- 입력은 유한한 0 이상의 값으로 정규화되며, p95는 항상 일반 값 이상으로 맞춰집니다.

### 한계
이 도구는 **결정론적 계획용 추정치이며 프로파일러가 아닙니다.** p95 합계는 모든 단계가 동시에 p95가 되는 보수적 상한입니다. 상관된 지터, 큐잉, 스케줄링, 전송 중첩, 하드웨어 경합은 반영하지 않으므로 실제 시스템에서 측정해야 합니다. 워커는 처리율만 높이며 한 프레임의 지연을 줄이지 않습니다.

### 조작법
- 표의 입력란을 수정하면 즉시 반영됩니다. 포커스를 벗어나면 값이 정규화됩니다.
- ↑ / ↓ 버튼으로 순서 변경, × 버튼으로 삭제, "+ 단계 추가"로 추가.
- 타임라인에 마우스를 올리거나, Tab으로 포커스한 뒤 화살표 키로 단계를 탐색합니다. Esc로 선택 해제.
- 모든 컨트롤은 키보드로 조작 가능하며, `prefers-reduced-motion`을 존중합니다.

### 로컬 빌드
```sh
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
cargo test
trunk build --release      # 결과물: dist/ (공개 경로 /latency-budget/)
bash tools/privacy_check.sh
```
`trunk serve`로 로컬 미리보기가 가능합니다. 앱은 `/latency-budget/` 하위 경로를 기준으로 빌드됩니다.

### 개인정보
- 100% 클라이언트 사이드: 서버, 분석, 트래커, 외부 폰트/CDN, 텔레메트리, 런타임 네트워크 요청이 없습니다.
- 입력값은 브라우저 안에만 머물며 저장·전송되지 않습니다.
- `index.html`의 CSP가 외부 요청을 차단합니다.

### 라이선스
MIT — [LICENSE](LICENSE) 참고.

---

## English

**Latency Budget** is a small static web app for robotics and VLA developers.
It answers: "Can this perception → policy → actuation pipeline meet its control-loop deadline, and which stage is the bottleneck?"
It is written in Rust and runs entirely in the browser as WebAssembly.

### Features
- **Editable pipeline budget** — six default stages (Sensor capture, Decode, Perception, Policy, Safety filter, Actuation). Edit each stage's name, typical latency (ms), p95 latency (ms) and parallel worker count (1–8); add, remove and reorder rows.
- **Timeline waterfall** — proportional stage blocks, typical vs p95 overlays, deadline marker, bottleneck highlight, tooltip and a status HUD.
- **Three illustrative presets** — Vision control, Local VLA policy, Remote-assisted policy. Synthetic, editable values; no vendor or product claims.
- **CSV export** — download the current budget as a local file.
- Korean by default with an English toggle.

### Model
- Sequential latency = sum of stage latencies (typical and p95 separately).
- Pipelined steady-state throughput = `1000 / max(typical / workers)` Hz. The stage that sets it is the **bottleneck** (first one on ties); its share of the sequential typical latency is shown.
- Margin = deadline − total (typical and p95).
- Status (boundaries inclusive):
  - **PASS**: p95 total ≤ deadline and max rate ≥ target Hz
  - **WARN**: typical total ≤ deadline but p95 total > deadline
  - **FAIL**: typical total > deadline, or max rate < target Hz
- Inputs are normalized to finite non-negative values; p95 is raised to at least typical.

### Limitations
This is a **deterministic planning estimate, not a profiler.** Summed p95 is a conservative bound that assumes every stage hits p95 at once. Correlated jitter, queueing, scheduling, transfer overlap and hardware contention are not modeled and must be measured on the real system. Workers raise throughput only; they do not shorten a single frame's latency.

### Controls
- Edit any field in the table; results update live and values are normalized when a field loses focus.
- ↑ / ↓ reorder, × removes, "+ Add stage" appends.
- Hover the timeline, or Tab to it and use the arrow keys to inspect stages; Esc clears.
- Every control is keyboard-accessible; `prefers-reduced-motion` is respected.

### Local build
```sh
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
cargo test
trunk build --release      # output: dist/ (public path /latency-budget/)
bash tools/privacy_check.sh
```
Use `trunk serve` for a local preview. The app is built for the `/latency-budget/` subpath.

### Privacy
- 100% client-side: no server, analytics, trackers, external fonts/CDNs, telemetry or runtime network requests.
- Entered values stay in your browser and are never stored or sent.
- The CSP in `index.html` blocks external requests.

### License
MIT — see [LICENSE](LICENSE).
