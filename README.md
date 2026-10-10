# robot_manager

VMX-pi에서 상시 실행되는 서비스. roboRIO의 로봇 프로그램 기동 구조(`robot` 서비스)를 대신한다.
표준 라이브러리만 쓰는 Python 3 스크립트 한 개(Python 3.10 이상, Ubuntu 22.04 기본 버전에서도 동작).

## 하는 일

- `/home/<user>/robotCommand`를 **root로** 실행하고, 끝나면 다시 실행한다. **실행 계약은 2027과 같은 `robotCommand`**
  (명령어가 한 줄 들어 있는 파일. `sh`로 실행하므로 shebang이 필요 없음). `frcUserProgram` 같은 고정 프로그램 이름은 요구하지 않는다.
- 아직 deploy가 안 돼서 `robotCommand`가 없으면 로그를 한 번 남기고 파일이 생길 때까지 기다린다.
- 크래시 반복 시 1초부터 최대 10초까지 간격을 늘려서 재시작한다. 10초 이상 돌았던 프로그램은 빠르게 다시 시작한다.
- 종료(`SIGTERM`/`SIGINT`) 시 로봇 프로그램의 **프로세스 그룹 전체**에 `SIGTERM`을 보내고, 5초 안에 안 죽으면 `SIGKILL`.
- **플랫폼 프로필**: 커널 버전(`platform.release()`)에 맞는 프로필의 환경변수를 로봇 프로그램에 넣어준다
  (예: `HALSIMVMX_DIO_MAP`). OS/커널별로 핀 번호가 달라질 수 있다는 정황이 있어 이 계층에서 흡수한다(아래 참고).
- **VMX를 직접 열지 않는다.** VMX(pigpio)는 로봇 프로그램이 연다. MockDS도 이 서비스의 역할이 아니다(사용자 코드).

## deploy와의 관계

deploy는 **GradleRIO가 SSH로 직접** 한다(2027과 동일). `robot_manager`는 deploy에 관여하지 않고 결과물만 실행한다.
원자적 교체/롤백/버전 보고는 실기기에서 deploy 문제(반쯤 올라간 상태, 크래시 루프)가 확인되면 그때 검토한다.

## 설정 (`config.json`)

`config.example.json`을 복사해서 쓴다. 기본 경로는 `/etc/robot_manager/config.json`.

```json
{
  "robot_home": "/home/<user>",
  "robot_command": "/home/<user>/robotCommand",
  "env": { "KEY": "value" },
  "profiles": [
    { "name": "...", "kernel_regex": "^6\\.", "env": { "HALSIMVMX_DIO_MAP": "0:12,1:13" } }
  ]
}
```

- `robot_home` (필수): GradleRIO `VmxPi`의 deploy 디렉터리(`/home/<username>`)와 같아야 한다. 로봇 프로그램의 작업 디렉터리.
- `robot_command` (선택): 기본값은 `<robot_home>/robotCommand`.
- 환경변수 우선순위: 서비스의 환경 < `env` < 일치한 프로필의 `env`. 값은 모두 문자열로 변환된다.
- 프로필은 위에서부터 보고 `kernel_regex`가 처음 일치하는 것 하나만 적용한다.

> **프로필 값은 비어 있다.** 핀 번호 오프셋이 OS/커널에 따라 필요할 수 있다는 것은 사용자 기억일 뿐 버전과 값을 모른다
> (미검증). 실기기에서 확인한 뒤, 어느 버전에서 얼마인지 근거와 함께 채울 것. 예시 파일의 값은 실제로 일치하지 않는다.

## 설치 (VMX-pi)

### 스크립트로 (`install.sh`)

로봇에서 이 레포와 `W4Studica/allwpilib`를 나란히 clone한 뒤 실행한다. ssh/비밀번호는 다루지 않는다(`sudo`가 직접 물어봄).

```bash
git clone https://github.com/W4Studica/Robot-Manager.git
git clone https://github.com/W4Studica/allwpilib.git
cd Robot-Manager
./install.sh --dry-run --user <deploy할 사용자>      # 먼저 무엇을 할지 확인
sudo ./install.sh --user <deploy할 사용자> --sudoers
```

`robot_manager` 설치, `studica_drivers` 빌드/설치, `libhalsim_vmx_studica.so` 빌드(`/opt/halsim_vmx/`), 선택적으로 passwordless sudo(`--sudoers`)를 한다.
필요한 것(VMXPi.h, g++ 등)이 없으면 그 단계만 건너뛴다. 옵션은 `./install.sh --help`. **실기기 미검증.**

### 수동으로

```bash
sudo mkdir -p /opt/robot_manager /etc/robot_manager
sudo cp robot_manager.py /opt/robot_manager/
sudo cp config.example.json /etc/robot_manager/config.json   # robot_home 수정
sudo cp robot_manager.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now robot_manager
journalctl -u robot_manager -f
```

GradleRIO `VmxPi`의 `serviceName` 기본값이 `robot_manager`라서 `./gradlew deploy`가 이 서비스를 stop/start 한다.
deploy하는 SSH 사용자는 `systemctl`과 `ldconfig`를 비밀번호 없이 `sudo`로 실행할 수 있어야 한다.

## 테스트

```bash
python3 -m unittest discover -s tests -v
```

13개 테스트: 프로필 선택/우선순위, shebang 없는 명령 실행, 환경변수 전달, 크래시 재시작, 파일이 없을 때 대기,
프로세스 트리 종료, `SIGTERM` 무시 프로그램 강제 종료, 실제 실행 파일의 `SIGTERM` 종료.

## 상태

PC에서만 테스트했다. systemd 유닛과 VMX 실기기 동작은 **미검증**.
