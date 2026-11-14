<p align="center"><img src="https://raw.githubusercontent.com/pcy200409/K-OMSI/branding/komsi/logo.png" alt="K-OMSI" width="470"></p>

**K-OMSI 0.4** - [openOMSI](https://github.com/openOMSI-org/openOMSI) 0.3.54 이후 개발판(main `0de28ab4`)을 바탕으로 K-OMSI 수정을 얹은 빌드입니다. 이번부터 K-OMSI 버전은 0.4.x로 따로 매깁니다.

> **비공식 포크입니다.** K-OMSI는 공식 openOMSI(openOMSI.org)와 관계없는 독립 프로젝트이며, openOMSI의 이름·로고는 원작자의 것입니다([TRADEMARKS.md](https://github.com/pcy200409/K-OMSI/blob/release/TRADEMARKS.md)). 문제는 공식 저장소가 아니라 [K-OMSI 이슈](https://github.com/pcy200409/K-OMSI/issues)로 알려 주세요.
>
> **라이선스: GNU AGPL v3 (`AGPL-3.0-only`)** - 공식 openOMSI를 따라 이번 릴리스부터 MIT에서 바뀌었습니다(예전 MIT 고지는 `LICENSE`에 남아 있음). 소스는 이 저장소에 모두 공개되어 있습니다.

> **초기 단계 프로젝트입니다. 버그가 있을 수 있습니다.** OMSI 2 정품이 설치되어 있어야 실행됩니다. 이 프로그램에는 게임 콘텐츠가 들어 있지 않습니다.

## 이번 빌드에 들어 있는 것

K-OMSI 단독 = K-OMSI에서만 더한 것, openOMSI 공식 = 공식 openOMSI에 들어 있는 것(0.3.54 정식판 이후 개발판에서 받은 변경). **(새)** = 이번 버전에서 처음 들어간 것.

| 기능 | 내용 | K-OMSI 단독 | openOMSI 공식 |
| --- | --- | :-: | :-: |
| 하차벨 어디서나 **(새)** | 버스 안 어디서든 하차 요청(Ctrl+Alt+S), LAN·시간표 버스 포함 | O | X |
| LED 행선판 낮 밝기 **(새)** | HTML LED 행선판이 낮에 더 밝고 창 너머에서도 빛번짐 | O | X |
| 한글 입력 **(새)** | LAN 채팅·플러그인·게임 패널 입력칸에서 한글 입력 | O | X |
| 느린 플러그인 다시 켜기 **(새)** | 50 ms 한 번 초과로 꺼지지 않음, 꺼진 플러그인 다시 켜기(Ctrl+Shift+L) | O | X |
| HTML 판 스크립트 여유 **(새)** | 큰 행선지 목록도 첫 실행에 만들고, 계속 넘치는 타이머만 멈춤 | O | X |
| 플러그인용 승객 API **(새)** | `people.hold/stay/leave/move/cancel/runner`, 승객 상황 플러그인용 | O | X |
| OBIS 노선팩·일시정지 유지 **(새)** | OBIS 클라이언트 없이 노선팩 직접 받기, 일시정지·포토모드 중에도 버스 유지 | O | X |
| 표시 이름·업데이트 | 창 제목·로딩 화면이 K-OMSI, 자동 업데이트는 이 저장소 릴리스 확인 | O | X |
| 텔레메트리 페달·주행거리 | 페달 값과 총 주행거리를 텔레메트리에 (외부 채점 도구용) | O | X |
| 승객 좌석 선택 | 짧게 타면 문 근처, 멀리 가면 안쪽, 노인은 빈 좌석에 | O | X |
| LAN 차량번호·번호판 | 다른 플레이어 버스가 그 화면과 같은 번호·번호판으로 보임 | O | X |
| AI 차선 변경 신호 | 차선 바꾸기 2초 전 방향지시등 | O | X |
| ReShade 호환 | ReShade를 넣으면 창이 뜨기 전에 꺼지던 문제 수정 | O | X |
| HTML 한글 글꼴 지정 | `fonts/html_fallback.ttf`로 HTML 페이지 한글 글꼴 지정 | O | X |
| 하이거 Auto 화면 (시험) | 하이거 1611P CCTV 화면의 안드로이드 오토 풍 차량 화면 ([설명](https://github.com/pcy200409/K-OMSI/blob/release/docs/android-auto.md)) | O | X |
| 맵 규칙 존중 | AI 속도는 맵 제한속도까지, `trafficdensity 0` 보도는 보행자가 피함 | O | X |
| AI 버스 하차벨 (LAN) | 사람 승객이 AI 버스에서 하차 요청하면 그 차의 하차벨이 울림 | O | X |
| 한국어 번역 보완 | 공식 번역에 없던 문구 977개 기계 번역 | O | X |
| LAN 받은 모드 지우기 | `OMSI_CLEAR_LAN_STORE=1`이면 남의 세션에서 받은 모드를 지움 | O | X |
| OBIS 연동 | OBIS 클라이언트가 있을 때 내 버스 송신·노선 띠 패널(Ctrl+Shift+O) | O | X |
| OBIS 노선 받아 오기 | 클라이언트에 없는 노선을 obis.kr에서 받아 옴 | O | X |
| TACO 운행기록계 | Light-TACO가 있을 때 운행기록계 패널·녹화(Ctrl+Shift+T) | O | X |
| C-ITS 경고 | 내비 옆 신호·추돌·급커브·단속카메라 알림 | O | X |
| 저지대 침수 (시험) | 폭우 때 저지대에 물이 차고 엔진 침수 (설정, 기본 꺼짐) | O | X |
| 블랙아이스 (시험) | 영하 근처 젖은 도로·다리가 얼어 미끄러움 (설정, 기본 꺼짐) | O | X |
| 앞차 물보라 | 젖은 길에서 앞차 물보라가 앞유리에 튐 | O | X |
| 포트홀 (시험) | 포장도로에 구멍, 폭우 뒤 더 많아짐 (설정, 기본 꺼짐) | O | X |
| 뇌우 정전 (시험) | 가까운 낙뢰로 신호등·가로등이 꺼짐 (설정, 기본 꺼짐) | O | X |
| 플레이어 버스 지붕 눈 | 지붕 높이에 맞춰 눈이 쌓임 | O | X |
| 한글 정류장명 | CP949 시간표를 한국어가 아닌 Windows에서도 한글로 읽음 | O | X |
| 저VRAM 보호 | 4 GB 이하 그래픽카드에서 텍스처를 압축된 채로 씀 | O | X |
| LAN 버전 불일치 안내 | 버전이 다를 때 어느 쪽이 구버전인지 한국어로 안내 | O | X |
| 미러 날씨 밝기 | 흐리거나 비 오는 낮에 미러만 어둡던 문제 수정 | O | X |
| 내비 첫 정류장 합류 | 이미 경로 위면 첫 정류장으로 되돌아가는 안내를 안 함 | O | X |
| 서버 맵 목록 누락 수정 | 한 번 플레이한 맵이 서버 목록에서 빠지던 문제 수정 | O | X |
| AI 후진 차선 (시험) | 맵의 `[rule] reverse` 차선을 AI가 후진으로 달림 | O | X |
| 셰이더 캐시 분리 | 런처 미리보기가 게임 셰이더 캐시를 덮어쓰지 않음 | O | X |
| 버스 빨리 불러오기 | 버스 모델을 여러 코어로 나눠 읽음 | O | X |
| AI 정체 줄이기 | 빨간불 앞 추월로 두 차로에 걸쳐 막히던 문제 등 수정 | O | X |
| 맵 전체 불러오기 | 시야 거리 "맵 전체 (무거움)", 시간표 버스가 맵 전체에서 다님 | O | X |
| 서버 메모리 줄이기 | `server.cfg`의 `stream_distance`로 플레이어 주변 타일만 불러옴 | O | X |
| 정류장 출발 추월 사고 수정 | 정류장 출발 버스를 뒤차가 추월하다 들이받던 문제 수정 | O | X |
| 승객 카드 태그 자세 | 승객이 단말기로 다가가 허리를 세우고 팔만 뻗어 찍음 | O | X |
| 도색 목록 갱신 | 덮어쓴 `.cti` 도색 이름이 런처에 바로 반영됨 | O | X |
| 텍스처 흐려짐 수정 | 메모리 한도 근처에서 텍스처가 계속 흐려지던 문제 수정 (#3888) | O | X |
| OMSI 2식 운전자 자세 | 운전자가 OMSI 2처럼 기대 앉아 핸들에서 손 뗌 (설정, 기본 꺼짐) | X | O |
| LAN 서버 주차 버스 | 서버에 보낸 버스가 주차되어 남고 다른 플레이어가 가져가 몰 수 있음 | X | O |
| 회차 대기 시간표 버스 | 회차 대기 중인 버스를 멈춘 차로 보고 지우던 문제 수정 | X | O |
| 블렌더 메시 뒤집힘 | 90° 회전 행렬 메시의 안쪽이 보이던 문제 수정 | X | O |
| BC 텍스처 없는 그래픽 칩 | 텍스처를 절반 크기로 올림 | X | O |
| 비트맵 글꼴 공백 | 비트맵 글꼴이 공백 글자를 그림 | X | O |
| AI 열차 행선 HOF | AI 열차 행선 텍스처의 HOF 선택 수정 | X | O |
| 장치 손실 튕김 | 그래픽 장치를 잃을 때 버퍼를 읽다 튕기던 문제 수정 (#3757) | X | O |
| 시간표 편집기 | 낮은 창에서 노선 목록이 접히던 문제 수정 | X | O |
| Android 미리보기 | 게임 중 미리보기가 렌더러·버스를 놓아 줌 | X | O |

맵 편집기 openMapper는 더 이상 함께 넣지 않습니다. K-OMSI 수정판은 https://github.com/pcy200409/K-openMapper 에서 따로 받으세요.

openOMSI의 지난 버전 변경 내역은 공식 [CHANGELOG](https://github.com/openOMSI-org/openOMSI/blob/main/CHANGELOG.md)(영어)를 보세요.

## 다운로드

| 플랫폼 | 파일 |
| --- | --- |
| Windows x64 | `openOMSI-<버전>-windows-x64.zip` (`openomsi.exe` 실행) |
| Windows ARM64 | `openOMSI-<버전>-windows-arm64.zip` |
| macOS (Apple silicon / Intel) | `-macos-arm64.zip` / `-macos-x64.zip` |
| Android (arm64, 8.0+) | `-android-arm64.apk` |
| 전용 서버 | `-server-windows-x64.zip`, `-server-windows-arm64.zip` |

> K-OMSI는 Linux 판(게임·전용 서버)을 만들지 않습니다. Linux에서는 소스로 직접 빌드해 주세요.
>
> 파일 이름이 `openOMSI-…`로 되어 있는 것은 자동 업데이트가 이 이름 형식으로 파일을 찾기 때문입니다.
