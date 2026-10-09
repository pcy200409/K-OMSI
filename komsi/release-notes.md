**K-OMSI** - [openOMSI](https://github.com/openOMSI-Project/openOMSI) 0.2.21을 바탕으로 K-OMSI 수정을 얹은 빌드입니다.

> **초기 단계 프로젝트입니다. 버그가 있을 수 있습니다.** OMSI 2 정품이 설치되어 있어야 실행됩니다. 이 프로그램에는 게임 콘텐츠가 들어 있지 않습니다.

## K-OMSI에서 더한 것

- **표시 이름**: 창 제목과 업데이트 안내가 K-OMSI로 표시됩니다. 자동 업데이트는 공식 openOMSI가 아니라 이 저장소의 릴리스를 확인합니다.
- **시간표 버스 보정** (`patch/ncc-route-gaps`): 분기점 앞에서 차선 변경, 시각이 정해지지 않은 중간 정류장의 대기 제한, 이어지지 않은 경로 구간을 건너뛰지 않도록 하는 수정
- **HTML 한글 폰트** (`patch/html-hangul-font`): HTML 텍스처에서 한글 폰트 사용
- **출발 정보 표시** (`patch/html-departure-details`): 정류장 표시기에 출발 상세 정보 추가
- **차고지 행선 문구** (`patch/depot-destination-strings`)
- **가로등 그림자 보정** (`patch/street-lamp-shadow-near`): Enhanced+에서 가로등 갓이 자기 불빛을 가려 발밑이 새까맣게 되는 문제 수정

## openOMSI 0.2.21 공식 변경사항 (요약)

### 새 기능
- **마우스 조향**: 조향 중에도 커서를 자유롭게 둘 수 있는 스위치 추가
- **파이프라인 캐시**: Vulkan/OpenGL에서 드라이버가 컴파일한 파이프라인을 저장해 두 번째 실행부터 시작이 빨라짐
- 런처가 있는 화면에서 게임이 열림 (전체화면 포함)

### 수정
- **시간표**: 등교일 운행이 다시 등교일에 운행됨 / 정류장 사이의 지연 표시가 버스 위치를 따라감
- **소리**: `[volume]`이 1을 넘는 항목이 처음부터 제 음량으로 재생됨 / AI 버스에 걸어서 탑승했을 때 내부 소리, 다른 플레이어 버스의 외부 소리 처리
- **AI 버스**: 문 앞 승객 때문에 정류장에서 영원히 출발하지 못하던 문제를 최대 1분 대기로 해결
- **자동 수동 변속기**: 저회전 디젤이 1단에 머무는 문제
- **Enhanced+ 반사**, **날씨**(연결 버스 뒷부분의 눈/젖음), **유리의 빗물**(와이퍼가 닿는 유리만 젖음)
- **상황 저장**: 행선표/IBIS/승차권 출력기 텍스처가 저장되고 복원됨
- **스크립트**: 잘못된 디스패치에 도달한 명령은 게임을 멈추지 않고 기록 후 건너뜀
- **조향 장치/컨트롤러**: 그래픽 태블릿 펜 조향, 래칭 스위치, Linux의 `BTN_TRIGGER_HAPPY` 장치
- **거울 패널(Ctrl+M)**, **런처 운행표 행**, **Android SD 카드 탐색**, **렌더러(Xclipse/ANGLE 그림자)**, **그래픽 드라이버(wgpu 포크 백포트)**, **Apple M4/Metal** 셰이더

공식 변경 내역 전문(영어): [CHANGELOG](https://github.com/openOMSI-Project/openOMSI/blob/main/CHANGELOG.md)

## 다운로드

| 플랫폼 | 파일 |
| --- | --- |
| Windows x64 | `openOMSI-<버전>-windows-x64.zip` (`openomsi.exe` 실행) |
| Windows ARM64 | `openOMSI-<버전>-windows-arm64.zip` |
| macOS (Apple silicon / Intel) | `-macos-arm64.zip` / `-macos-x64.zip` |
| Linux x64 / ARM64 | `-linux-x64.zip` / `-linux-arm64.zip` |
| Android (arm64, 8.0+) | `-android-arm64.apk` |
| 전용 서버 | `-server-linux-x64.zip`, `-server-linux-arm64.zip`, `-server-windows-x64.zip`, `-server-windows-arm64.zip` |

> 파일 이름이 `openOMSI-…`로 되어 있는 것은 자동 업데이트가 이 이름 형식으로 파일을 찾기 때문입니다.
