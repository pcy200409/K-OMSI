# K-OMSI에 기여하기

> K-OMSI는 [openOMSI](https://github.com/openOMSI-Project/openOMSI)를 바탕으로 한 빌드이며, 이 문서는 openOMSI의
> [CONTRIBUTING.md](https://github.com/pcy200409/K-OMSI/blob/release/CONTRIBUTING.md)를 한국어로 옮긴 것입니다.
> 영어 원문과 다를 경우 원문이 우선합니다. 엔진 자체의 수정은 가능하면 공식 openOMSI에도 함께 제안해 주세요.

도와주셔서 감사합니다! 몇 가지 규칙이 프로젝트를 건강하게 유지합니다.

* **원본 코드와 에셋을 넣지 않습니다.** OMSI 2 설치본에서 어떤 것도 저장소로 복사하지 마세요.
  텍스처, 모델, 사운드, 맵, 스크립트 모두 해당합니다. 콘텐츠가 필요한 테스트는 로컬 설치본(`OMSI_ROOT`)에서
  읽고, 설치본이 없으면 스스로 건너뜁니다.
* **호환성이 먼저입니다.** 기본 맵이나 지금까지 잘 되던 모드를 깨뜨리는 변경은 안 됩니다. 큰 변경 전후에
  `cargo run --release -p omsi-check -- "/path/to/OMSI 2"`를 실행해 보세요.
* **풀 리퀘스트 하나에는 변경 하나.** 설명에는 플레이어가 무엇을 느끼게 되는지를 적어 주세요.
* `cargo test --workspace`와 `cargo build --release`가 통과해야 합니다. (CI가 모든 플랫폼을 확인합니다.)
* 코드 스타일: `rustfmt` 기본값을 쓰고, 주석에는 *왜* 그렇게 했는지를 적습니다.

## 파일과 라이선스

* **기여한 내용은 MIT 라이선스**입니다. 나머지 코드, 문서와 같습니다. 게임에 포함된 일부 외부 파일
  (Roboto 글꼴, `LICENSE.txt`가 딸린 Material 아이콘)은 각자의 라이선스를 유지합니다. 다른 것을 추가하려면
  (OpenStreetMap 지도 데이터(ODbL), CC-BY-SA 그림, 다른 글꼴이나 아이콘 세트 등) 먼저 관리자의 동의가
  필요합니다. 형식은 여기에 문서화하고 실제 데이터는 다른 곳(릴리스, 별도 저장소, 모드 자체)에 올릴 수
  있습니다.
* **큰 파일은 안 됩니다.** 1MB 이상의 파일은 PR 검사에서 실패합니다. 스크린샷과 영상은 커밋이 아니라
  풀 리퀘스트 설명에 올리세요. 테스트 데이터는 작게, 그 테스트를 위해 만든 것만 넣습니다.
* **생성되거나 내려받은 콘텐츠는 안 됩니다.** 빌드 결과물, 캐시, 변환한 텍스처, 예제 맵을 넣지 마세요.

## 번역

화면 문구는 [`crates/omsi-app/locales/app.yml`](https://github.com/pcy200409/K-OMSI/blob/release/crates/omsi-app/locales/app.yml)에 있으며
영어 문장이 키입니다. 이 파일을 바꾸는 풀 리퀘스트는 자동으로 검사됩니다.

* 내 언어의 문구를 추가하거나 고치세요. 키에 들어 있는 자리표시자(`%{packs}`, `{}`)는 그대로 둡니다.
* 키를 지우거나 영어 문장을 고쳐 쓰지 마세요. 코드가 그 문장으로 문구를 찾습니다.
* 다른 기여자의 기존 번역을 고치는 것은 오류를 바로잡는 경우라면 괜찮습니다. 설명에 그렇게 적어 주세요.
  검사가 바뀐 모든 문구를 리뷰어에게 보여 줍니다.

## 이슈

* **영어로만 작성합니다.** (공식 openOMSI의 규칙입니다.) 제목과 본문을 영어로 써야 모든 기여자가 읽고 검색할
  수 있습니다. 다른 언어로 쓴 이슈는 번역해 달라는 안내와 함께 자동으로 닫히고, 영어로 고치면 저절로 다시
  열립니다. 로그와 게임 안의 문구는 그대로 둬도 됩니다.
* 이슈 하나에는 문제나 아이디어 하나, 그리고 최신 릴리스 기준으로 써 주세요. 질문은
  [Discord 서버](https://discord.gg/VG2EKVafYG)에서 해 주세요.
* 새 이슈는 자동으로 분류됩니다. 양식의 "What is it about?"가 `area:` 라벨이 되고 첫 마일스톤이
  붙습니다. (회귀나 크래시는 v0.1.x) 더 맞는 곳이 있으면 관리자가 옮깁니다. 열려 있는 이슈와 같은 패닉의
  크래시 보고는 중복으로 닫힙니다. 그 이슈에 당시 상황을 덧붙여 주세요.
* **보안 문제는 이슈로 올리지 않습니다.** [SECURITY.md](https://github.com/pcy200409/K-OMSI/blob/release/.github/SECURITY.md)를 보세요.

## 어디에 무엇이 있나

구조는 [README](https://github.com/pcy200409/K-OMSI/blob/release/README.md#repository-layout)와
[docs/ARCHITECTURE.md](https://github.com/pcy200409/K-OMSI/blob/release/docs/ARCHITECTURE.md)를 보세요.
파일 형식은 [docs/FORMATS.md](https://github.com/pcy200409/K-OMSI/blob/release/docs/FORMATS.md)에 있습니다.

## 릴리스

관리자가 `VERSION` 파일의 `MAJOR.MINOR`를 올립니다. 나머지는 모두 자동입니다.
[docs/VERSIONING.md](https://github.com/pcy200409/K-OMSI/blob/release/docs/VERSIONING.md)를 보세요.
