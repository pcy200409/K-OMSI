# K-OMSI에서 DLSS · ReShade 쓰기

K-OMSI(openOMSI)에서 NVIDIA DLSS 계열 기능을 쓰는 방법은 두 가지입니다. **둘을 동시에 켜지 마세요.** 하나씩 써 보고 마음에 드는 쪽을 고르면 됩니다.

| | ① 게임 내장 DLSS | ② ReShade + RenoDX DLSS 5 |
|---|---|---|
| 무엇 | 게임이 직접 DLSS(Super Resolution/DLAA)를 부름 | ReShade 애드온이 완성된 화면에 DLSS 5 신경망 렌더링을 입힘 |
| 해상도 | 작게 그려 키움(품질~울트라 성능) 또는 DLAA(원래 해상도) | 보통 원래 해상도 그대로(`NREnableUpscaling=0`) |
| 움직임 정보 | 게임이 물체별 움직임·깊이를 직접 넘겨줌 | 애드온이 받아 낼 수 있는 만큼 |
| 필요한 것 | NVIDIA Streamline DLL 4개 | ReShade(애드온 지원판) + RenoDX DLSS5 애드온(DLSS5-Swapper로 설치) |
| 상태 | 시험 기능 | 시험 단계(K-OMSI에서 아직 충분히 확인 안 됨) |

공통 조건: Windows, NVIDIA GeForce RTX, 그래픽 API **DirectX 12**.

> K-OMSI는 ReShade·NVIDIA 파일을 함께 배포하지 않습니다. 각자 공식 배포처에서 받으세요. 직접 넣은 파일은 K-OMSI 자동 업데이트가 지우지 않습니다(업데이트는 자기가 설치한 파일만 바꿉니다).

---

## ① 게임 내장 DLSS

1. [NVIDIA Streamline SDK](https://github.com/NVIDIA-RTX/Streamline) 릴리스를 받아 `bin/x64`에서 아래 4개를 `openomsi.exe`가 있는 폴더에 복사합니다.
   `sl.interposer.dll`, `sl.common.dll`, `sl.dlss.dll`, `nvngx_dlss.dll`
2. 런처 → 설정 → 그래픽 → 안티에일리어싱 아래 **DLSS**에서 고릅니다.
   - **DLAA**: 원래 해상도 그대로, 안티에일리어싱만 DLSS로(가장 선명)
   - **품질 / 균형 / 성능 / 울트라 성능**: 가로·세로 약 67 / 58 / 50 / 33 %로 그려 키움(빠름)
3. 게임을 시작합니다(DLSS를 켜면 DirectX 12가 자동으로 잡힙니다). 품질은 게임 안 ESC → 옵션 → 그래픽에서 바로 바꿀 수 있고, 켜기·끄기는 다음 실행부터 적용됩니다.
4. 확인: `C:\Users\<사용자>\.openomsi\game.log`에 `DLSS … on:` 줄이 있으면 켜진 것입니다. `DLSS cannot start: …`면 그 뒤에 이유가 적혀 있고, 게임은 DLSS 없이 평소대로 그립니다.

알려진 점
- 품질~울트라 성능에서 먼 표지판 글씨가 흐릴 수 있습니다(작게 그린 화면의 텍스처 단계 보정이 아직 없음). DLAA에서는 그렇지 않습니다.
- 가장자리가 반짝이거나 번지면 환경변수 `OMSI_DLSS_JITTER_SIGN=-1`로 실행해 보세요.

---

## ② ReShade + RenoDX DLSS 5 (DLSS5-Swapper)

원래 OMSI 2(32비트 DirectX 9)에서는 DLSS5-Swapper가 64비트 도우미(`dlss5-feed-host64.exe`)를 띄워 화면을 넘겨받는 방식이었습니다. K-OMSI는 64비트 DirectX 12 게임이라 그 우회가 필요 없고, 게임 폴더에 바로 설치합니다.

1. **K-OMSI 버전 확인**: **0.2.89 이상**이어야 합니다(ReShade 호환 수정과 게임 내장 DLSS가 이 버전부터 들어 있습니다). 이전 버전은 ReShade를 넣으면 창이 뜨기 전에 꺼집니다.
2. **그래픽 API를 DirectX 12로**: 런처 → 설정 → 그래픽 API → DirectX 12. ①의 게임 내장 DLSS는 **끔**으로 둡니다.
3. **[DLSS5-Swapper](https://github.com/rakanki911/DLSS5-Swapper/releases)** (MIT)를 설치하고 실행합니다.
4. 게임으로 K-OMSI 폴더(`openOMSI-<버전>-windows-x64`)의 **`openomsi.exe`**를 고릅니다. 그래픽 API는 자동 감지되며, 안 맞으면 DirectX 12로 직접 지정합니다.
5. DirectX 12 게임의 방식으로 **ReShade / Feeder**를 고르고 적용합니다. Swapper가 ReShade(애드온 지원판, 6.8.0)와 RenoDX DLSS5 애드온(`renodx-dlss5.addon64`), 필요한 DLSS 파일을 받아 넣습니다. Visual C++ 런타임(x64)이 필요합니다.
6. 게임을 실행합니다.
   - **Home**: ReShade 화면 열기(애드온 탭의 "DLSS 5 Neural Rendering" 패널)
   - **F6**: DLSS 5 켜기/끄기, **F5**: 스크린샷
   (OMSI 2에서 쓰던 `ReShade.ini` 설정 기준입니다. Swapper 기본값과 다를 수 있습니다.)

### NVIDIA 드라이버 주의

DLSS5-Swapper의 도우미 로그에 다음 경고가 있습니다(요약): **RenoDX DLSS5 v4.7 이상 + NVIDIA 드라이버 616.64 이상**에서는 신경망 처리가 드라이버 안에서 실패해 DLSS 5가 아무것도 내놓지 않을 수 있다. 동작하는 조합으로는 드라이버 **616.56**, "Deep Fried Chicken" 소비자, 또는 v4.6 미만 빌드가 적혀 있습니다. DLSS 5가 켜졌는데 화면에 변화가 없으면 이것부터 의심하세요(도우미를 `--test`로 실행하면 조합을 바로 확인할 수 있다고 합니다).

### 문제가 생기면

- **게임이 창도 안 뜨고 꺼짐**: K-OMSI 버전이 오래된 것입니다(1번). 또는 ReShade를 DirectX 9/11용으로 넣었는지 확인하세요.
- **로그**: `openomsi.exe` 옆의 `ReShade.log`, 게임의 `C:\Users\<사용자>\.openomsi\game.log`.
- **되돌리기**: Swapper에서 제거하거나, `openomsi.exe` 옆의 `dxgi.dll`과 `ReShade.ini`, `*.addon64`를 지웁니다.
