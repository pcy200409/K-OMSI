<h1 align="center">K-OMSI</h1>

<p align="center">
  Rust로 새로 만든 버스 시뮬레이터 <b>OMSI 2</b> 호환 엔진, <a href="https://github.com/openOMSI-Project/openOMSI">openOMSI</a> 기반 한국판 빌드
</p>

<p align="center">
  <a href="https://github.com/pcy200409/K-OMSI/blob/release/LICENSE"><img alt="License" src="https://img.shields.io/github/license/pcy200409/K-OMSI?style=for-the-badge"></a>
  <a href="https://github.com/openOMSI-Project/openOMSI"><img alt="Based on openOMSI" src="https://img.shields.io/badge/based%20on-openOMSI-f47f30?style=for-the-badge"></a>
</p>

> [!WARNING]
> **초기 단계의 프로젝트입니다. 버그가 있을 수 있습니다.** K-OMSI는 openOMSI를 바탕으로 하며,
> openOMSI 자체가 아직 개발 초기라 기능이 빠져 있거나 버전마다 바뀔 수 있습니다.
> 문제는 [Issues](https://github.com/pcy200409/K-OMSI/issues)에 남겨 주세요.

## K-OMSI란?

**K-OMSI**는 [openOMSI](https://github.com/openOMSI-Project/openOMSI)(MIT 라이선스)에
몇 가지 수정을 더한 빌드입니다. 공식 openOMSI에 새 버전이 나오면 그 변경을 그대로 받아오고,
K-OMSI에서 만든 수정은 새 버전 위에 다시 얹어서 계속 유지합니다.

openOMSI는 버스 시뮬레이터 **OMSI 2**를 Rust로 처음부터 다시 만든 프로젝트입니다. 64비트와
멀티스레드를 지원하고, 현대적인 렌더러(wgpu를 통한 Metal / Vulkan / DirectX 12)를 쓰며,
기존 OMSI 2의 맵, 버스, 오브젝트, 모드와 그대로 호환됩니다.

> [!IMPORTANT]
> **OMSI 2 정품이 설치되어 있어야 합니다.** 이 프로그램에는 게임 콘텐츠가 들어 있지 않습니다.
> 설치된 OMSI 2의 맵과 차량 등을 불러와서 실행하므로, 정품 없이는 **실행되지 않습니다.**

## openOMSI와 다른 점

공식 openOMSI 위에 다음 내용이 더해져 있습니다.

| 구분 | 내용 |
| --- | --- |
| 표시 이름 | 창 제목과 업데이트 안내 문구가 K-OMSI로 표시됩니다. |
| 업데이트 | 자동 업데이트가 공식 openOMSI가 아니라 **이 저장소의 릴리스**를 확인합니다. 공식판으로 덮어써지지 않습니다. |
| 시간표 버스 보정 (`patch/ncc-route-gaps`) | 분기점 앞에서 차선 변경, 시각이 정해지지 않은 중간 정류장에서의 대기 제한, 이어지지 않은 경로 구간을 건너뛰지 않도록 하는 수정 |
| HTML 한글 폰트 (`patch/html-hangul-font`) | HTML 텍스처에서 한글 폰트 사용 |
| 출발 정보 표시 (`patch/html-departure-details`) | 정류장 표시기에 출발 상세 정보 추가 |
| 차고지 행선 문구 (`patch/depot-destination-strings`) | 차고지 행선 문구 보강 |
| 가로등 그림자 보정 (`patch/street-lamp-shadow-near`) | Enhanced+에서 가로등 갓이 자기 불빛을 가려 발밑이 새까맣게 되는 문제 수정 |

## 다운로드

> [!NOTE]
> 아직 K-OMSI용 릴리스 파일은 올라와 있지 않습니다. 당분간은 아래 [소스에서 빌드](#소스에서-빌드)로 직접
> 만들어 쓰셔야 합니다. 릴리스가 올라오면 [Releases](https://github.com/pcy200409/K-OMSI/releases)에서 받을 수 있습니다.

## 설치와 실행

**먼저 OMSI 2가 설치되어 있어야 합니다.** (Steam 또는 패키지 버전, 어느 버전이든 상관없고 기본 콘텐츠가
있어야 합니다. Grundorf와 Berlin-Spandau 맵, 기본 버스 MAN SD200/SD202, NL 등)

1. 내 시스템에 맞는 파일을 받아 **쓰기 가능한 폴더**에 풀어 주세요. 문서 폴더, 게임 폴더, OMSI 2 폴더
   안 모두 가능합니다. `Program Files`는 피해 주세요. 자동 업데이트가 거기서는 동작하지 않습니다.
2. 실행합니다.
   * **Windows:** `openomsi.exe`. SmartScreen이 경고하면 *추가 정보* → *실행*을 누릅니다.
   * **macOS:** `openOMSI.app`을 엽니다. 처음에는 우클릭 → *열기*가 필요할 수 있습니다.
   * **Linux:** `./openomsi`. Vulkan 또는 OpenGL 드라이버가 필요합니다.
3. **OMSI 2 폴더를 지정합니다.** 런처가 대개 설치 위치를 자동으로 찾습니다. 못 찾으면 **Setup**에서
   `Omsi.exe`, `maps`, `Vehicles`가 들어 있는 OMSI 2 폴더를 고르고 **Save**를 누르세요.
   Steam 버전은 보통 `…\Steam\steamapps\common\OMSI 2`에 있습니다.
4. **운행:** **Drive** 페이지에서 버스, 맵, 운행표를 고르고 **Start the duty**를 누릅니다.

모드는 런처의 **Mods** 페이지에서 설치하거나(폴더, `.zip`, 창에 끌어놓기), 게임 옆의 `Mods` 폴더에
직접 넣으면 됩니다. 원본 OMSI 2 폴더에는 아무것도 쓰지 않습니다.

### 문제가 생겼을 때

* **"The original OMSI 2 was not found"**: Setup에서 OMSI 2 폴더를 다시 지정하세요. 메시지에 그
  폴더에서 빠져 있는 것이 적혀 있습니다.
* **몇 초 뒤에 게임이 꺼지거나 "graphics device was lost"가 나올 때**: 그래픽 드라이버를 제조사
  (NVIDIA, AMD, Intel) 것으로 업데이트하세요. Windows에서는 Settings → Graphics → Graphics API에서
  DirectX 12로 바꿔 볼 수 있습니다.
* **오래된 그래픽 카드(Vulkan 미지원)**: DirectX 12, 그다음 OpenGL로 자동 전환됩니다.
* **모드 맵에서 다리나 보이지 않는 벽에 걸릴 때**: Esc → Options → *Collisions with objects*를 끄세요.
* **멀티플레이에서 서로 안 보일 때**: 두 사람 모두 호스트의 맵이 있어야 합니다. OMSI 2 폴더의 맵은
  전달되지 않고, Mods 페이지로 설치한 맵은 전달됩니다.
* **그 밖의 문제**: 게임이 오류로 끝나면 런처에 *Copy report*가 나옵니다. 로그는
  `C:\Users\<이름>\.openomsi`의 `game.log`에 있고, Setup의 *Export diagnostics*로 지원용 파일을 만들 수
  있습니다. 이 파일에는 폴더 경로, 이름, 채팅이 들어가지 않으며 자동으로 전송되지 않습니다.

## 소스에서 빌드

```sh
git clone https://github.com/pcy200409/K-OMSI.git && cd K-OMSI
scripts\build-windows.cmd     # Windows → dist\windows\openomsi.exe
scripts/build-linux.sh        # Linux   → dist/linux/openomsi
scripts/build-macos.sh        # macOS   → dist/macos/openOMSI.app
```

최신 [Rust stable](https://rustup.rs)과 각 플랫폼의 C 도구 체인이 필요합니다. Windows에서는
[CMake](https://cmake.org/)도 필요합니다. 자세한 내용은 [docs/BUILDING.md](https://github.com/pcy200409/K-OMSI/blob/release/docs/BUILDING.md)를
보세요.

## 브랜치 구조와 openOMSI 업데이트 따라가기

| 브랜치 | 내용 |
| --- | --- |
| `release` | 실제로 빌드해서 쓰는 브랜치입니다. 아래 브랜치들을 합친 결과입니다. 기본 브랜치입니다. |
| `main` | 공식 openOMSI와 **똑같은** 복사본입니다. 수정하지 않고 업데이트만 받습니다. |
| `branding` | 표시 이름, 업데이트 대상, 동기화 스크립트 |
| `patch/*` | K-OMSI의 수정 사항 (기능별로 하나씩) |

공식 openOMSI에 새 버전이 나오면 `komsi/sync-upstream.ps1`이 `main`을 갱신하고, 각 패치를 새 버전
위에 다시 얹은 뒤 `release`를 새로 만듭니다. 공식이 이미 받아들인 수정은 자동으로 빠집니다.
패치 목록은 `komsi/patches.txt`에 있습니다.

```powershell
.\komsi\sync-upstream.ps1 -Check   # 공식에 무엇이 새로 올라왔는지, 어느 패치와 겹치는지만 확인
.\komsi\sync-upstream.ps1 -Build   # 반영하고 테스트와 컴파일까지 확인
```

## 문서

기본 문서는 openOMSI의 것을 그대로 쓰며 영어입니다. [docs/](https://github.com/pcy200409/K-OMSI/tree/release/docs)에 사용 설명서,
모드 제작, 파일 형식, 플러그인, 전용 서버 등이 있습니다.
공식 웹사이트: https://openomsi-project.github.io/openOMSI/

## 라이선스와 출처

K-OMSI는 [openOMSI](https://github.com/openOMSI-Project/openOMSI)(Copyright (c) 2026 usonskyyyy 외
기여자들)를 바탕으로 한 파생 빌드이며, 동일하게 [MIT 라이선스](https://github.com/pcy200409/K-OMSI/blob/release/LICENSE)로 배포됩니다.
([한국어 참고 번역](https://github.com/pcy200409/K-OMSI/blob/release/LICENSE.ko.md), 법적 효력은 영어 원문에 있습니다.)
기여 방법은 [CONTRIBUTING](https://github.com/pcy200409/K-OMSI/blob/release/.github/CONTRIBUTING.md), 보안 제보는
[SECURITY](https://github.com/pcy200409/K-OMSI/blob/release/.github/SECURITY.md)를 보세요.
OMSI와 OMSI 2는 각 권리자의 상표입니다. K-OMSI는 독립 프로젝트이며 OMSI 2의 권리자나
openOMSI 프로젝트와 공식적인 관계가 없습니다.

openOMSI가 마음에 드셨다면 원작자를 후원해 주세요:
[Buy me a coffee](https://buymeacoffee.com/usonskyyy) · [Ko-fi](https://ko-fi.com/usonance)
