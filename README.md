# robot_manager

VMX-pi에서 상시 실행되는 서비스. roboRIO의 로봇 프로그램 기동 구조(`robot` 서비스)를 대신한다.
표준 라이브러리만 쓰는 Python 3 스크립트 한 개(Python 3.10 이상, Ubuntu 22.04 기본 버전에서도 동작).

## 하는 일

- `/home/<user>/robotCommand`를 **root로** 실행하고, 끝나면 다시 실행한다. **실행 계약은 2027과 같은 `robotCommand`**
  (명령어가 한 줄 들어 있는 파일. `sh`로 실행하므로 shebang이 필요 없음). `frcUserProgram` 같은 고정 프로그램 이름은 요구하지 않는다.
- 아직 deploy가 안 돼서 `robotCommand`가 없으면 로그를 한 번 남기고 파일이 생길 때까지 기다린다.
- 크래시 반복 시 1초부터 최대 10초까지 간격을 늘려서 재시작한다. 10초 이상 돌았던 프로그램은 빠르게 다시 시작한다.
- **종료 순서**(서비스 stop 또는 `SIGTERM`/`SIGINT`): 로봇 프로그램의 **프로세스 그룹 전체**에 `stop_signals`를 순서대로 보낸다(기본 **`SIGINT` -> `SIGTERM`**). 각 신호에 `stop_grace_seconds`(기본 5초)의 균등 몫을 주고, 리더가 끝나면 거기서 멈춘다. 다 지나도 안 죽으면 그룹에 `SIGKILL`.
  **왜 `SIGINT`가 먼저인가(개발용 VMX에서 HAL만 초기화하고 대기하는 시험 프로그램으로 측정):** VMX HAL은 `SIGINT`를 받으면 pigpio를 정리하고 약 **0.2초 만에 종료**한다(`Exiting VMX-pi HAL application ... [normal exit]`). `SIGTERM`은 pigpio를 정리하지만 **프로세스를 끝내지 않고 돌아온다**(`gdb`로 확인: 메인 스레드가 그대로 대기 루프). `SIGTERM`만 보내면 매번 유예 시간을 다 쓰고 `SIGKILL`로 끝난다.
  HAL이 종료 때 찍는 `Signal 2 received by PIGPIOClient::signal_func()`, 스택 출력, `Exiting ... [normal exit]`는 정상 출력이다(옛 VMX 배포에서도 매번 보이던 메시지로 알려져 있으나 그 신호를 소스로 확인하지는 못했다. 옛 포크는 `frcKillRobot.sh -t`를 호출하고, 그 스크립트는 VMX 이미지 안에 있다).
  리더 프로세스가 끝난 뒤에도 그룹에 남은 프로세스는 **남은 유예 시간까지 기다린 뒤** `SIGKILL`로 정리한다. 프로그램이 스스로 끝났는데(크래시 등) 자식이 남아 있으면 그 자식에게 먼저 `SIGTERM`, 유예 후 `SIGKILL`.
  종료할 때마다 `robot program stopped by SIGINT after 0.2s` / `robot program killed after 5.0s`를 로그에 남기고, `SIGKILL`이 필요했다면 경고한다(프로그램의 정리 코드, 예를 들면 pigpio의 `gpioTerminate`가 돌지 않았다는 뜻이다).
  서비스(`robot_manager`)는 종료 스레드가 끝날 때까지 기다린 뒤에 종료한다(그렇지 않으면 그룹 정리가 끊기고 systemd가 15초 뒤 대신 `SIGKILL`한다. 로봇에서 실제로 겪었다).
- **플랫폼 프로필**: 커널 버전(`platform.release()`)에 맞는 프로필의 환경변수를 로봇 프로그램에 넣어준다
  (예: `HALSIMVMX_DIO_MAP`). OS/커널별로 핀 번호가 달라질 수 있다는 정황이 있어 이 계층에서 흡수한다(아래 참고).
- **VMX를 직접 열지 않는다.** VMX(pigpio)는 로봇 프로그램이 연다. MockDS도 이 서비스의 역할이 아니다(사용자 코드).

## GPIO 인터럽트 shim (`gpio_isr_shim/`)

sysfs GPIO가 없는 커널(예: Ubuntu 26.04 / 커널 7.0)에서 VMX HAL이 `RPI GPIO Interrupt Enable:  PI_BAD_ISR_INIT.`를 내는 문제를 고치는 `LD_PRELOAD` 라이브러리다.
`install.sh`가 빌드해서 `/opt/robot_manager/lib/libvmx_gpio_isr_shim.so`에 설치하고, 로봇 프로그램의 환경에는 `config.json`의 `env`로 준다:

```json
{ "env": { "LD_PRELOAD": "/opt/robot_manager/lib/libvmx_gpio_isr_shim.so" } }
```

자세한 내용과 한계는 [gpio_isr_shim/README.md](gpio_isr_shim/README.md). **PC에서 콜백 규약만 테스트했고 실기기 검증 전이다.**

## LSE 원자 명령 에뮬레이션 (`lse_emu/`)

WPILib이 `linuxarm64`로 배포하는 `libMrcLib.so`(Driver Station/로봇 통신, 소스 비공개)는 **SystemCore 툴체인(Buildroot GCC 14.3)으로 빌드돼서 ARMv8.1 LSE 원자 명령**(`swp`, `ldadd`, `cas`)을 코드 곳곳에 쓴다(1,219곳). Pi 4(Cortex-A72, ARMv8.0)에는 이 명령이 없어서 **`SIGILL`**이 난다. 로봇에서 실측: 프로세스 종료 때 `uv_library_shutdown()`에서 죽어 종료 코드 132와 코어 덤프가 매번 생겼다. 다른 WPILib 라이브러리와 우리 코드에는 CPU를 확인해서 쓰는 libgcc 보조 함수(`__aarch64_*`)만 있어 문제가 없다.

`lse_emu/`는 `SIGILL`을 잡아 그 명령을 같은 동작의 일반 원자 연산(`ldxr/stxr`)으로 대신 실행하고 다음 명령으로 넘어가는 라이브러리다(원자성 유지, 명령마다 신호 한 번 분의 비용). **`LD_PRELOAD`가 아니라 `LD_AUDIT`으로 올려야** 한다: 로더는 라이브러리를 목록 뒤쪽부터 초기화해서, 프리로드한 라이브러리는 MrcLib의 생성자보다 늦게 초기화되기 때문이다(측정함).

`robot_manager`가 `/proc/cpuinfo`의 `Features`에 `atomics`가 없으면 **스스로 `LD_AUDIT`을 설정**한다(`install.sh`가 `libvmx_lse_emu.so`를 설치). 설정: `"lse_emulation": "auto"`(기본) / `"off"` / `"on"`, `"lse_emulation_library"`. `env`에 이미 `LD_AUDIT`이 있으면 건드리지 않는다. 자세한 내용은 [lse_emu/README.md](lse_emu/README.md).

## 강제 종료(SIGKILL)와 pigpio

`robot_manager`는 로봇 프로그램을 `SIGINT` -> `SIGTERM` -> `SIGKILL` 순서로 멈춘다(`stop_signals`). 프로그램이 `SIGKILL`로 죽으면(`stop_grace_seconds` 안에 안 끝났을 때, 또는 누군가 `kill -9` 했을 때) pigpio가 GPU 메모리를 정리하지 못해서 **새어 나가고**, 몇 번 쌓이면 다음 시작이 `initMboxBlock: init mbox zaps failed`로 실패한다(로봇에서 측정). 재부팅하면 풀린다. 모터는 이 경우에도 멈춘다: `TitanEnableGuard`/`StudicaTitan`(allwpilib `halsim_vmx`)이 enable 프레임을 반복 송신에 맡기지 않아서, 프로그램이 죽으면 Titan이 200 ms 안에 스스로 멈춘다.

## 네트워크 (`net/`, Studica VMX 이미지와 같은 방식)

대회처럼 개발 PC가 로봇에 직접 붙을 수 있게 한다. 명령 이름과 사용법은 Studica VMX 이미지의 것과 같다.

| 방식 | VMX의 주소 | 설명 |
|---|---|---|
| **Wi-Fi AP** (기본) | **`10.XX.YY.2`** (팀 1234 -> `10.12.34.2`, 4444 -> `10.44.44.2`) | VMX가 AP와 DHCP 서버가 된다. 이미지 기본값: SSID `VMX-1234`, 비밀번호 `password`, 2.4GHz, 채널 1 |
| **유선** (항상 켜짐) | **`172.22.11.2`** (고정) | 직접 연결한 PC는 VMX의 DHCP로 `172.22.11.XX`를 받는다. 되도록 유선을 쓴다 |
| Wi-Fi 클라이언트 | 공유기가 줌 | AP를 끄고 공유기에 접속. 업데이트/패키지 다운로드용 |

```bash
sudo setupHotspot VMX 1234 2.4 password          # SSID접두사 팀번호 밴드 [비밀번호]; 채널은 물어본다(기본 6)(--channel N)
sudo setupHotspot TestRobot 4444 2.4 Checkers    # TestRobot-4444, 10.44.44.2
sudo toggleNetMode wifi HomeRouter routerpass    # 공유기에 접속 (AP는 꺼짐; 이후 `toggleNetMode wifi`만으로 다시)
sudo toggleNetMode ap                            # 다시 AP
sudo setupEthernet                               # 유선 172.22.11.2 (DHCP 서버 포함)
```

`install.sh`는 이 명령들을 항상 설치하고, **`--network`를 줄 때만** 적용한다(`--team`, `--ap-ssid`, `--ap-band`, `--ap-password`로 값 변경).

**`--network`가 하는 일 (Ubuntu Server는 NetworkManager가 아니라 systemd-networkd + netplan이라서):**
1. `network-manager`, `dnsmasq-base`, `iw` 설치(인터넷 필요).
2. 유선 `172.22.11.2`와 AP 연결 파일을 `/etc/NetworkManager/system-connections/robot-*.nmconnection`(모드 0600)에 씀.
3. 기존 `/etc/netplan/*.yaml`을 `/etc/netplan/w4studica-backup-<시각>/`로 **치워 두고**(지우지 않음) renderer만 NetworkManager인 netplan 파일을 씀. cloud-init이 부팅 때 다시 쓰지 못하게 끔.
4. **스크립트가 끝난 8초 뒤에** NetworkManager로 전환(`systemd-networkd` 중지, `NetworkManager` 시작). 이때 **SSH 세션이 끊긴다**(집 Wi-Fi 설정은 더 이상 쓰지 않음).
5. 다시 접속: **랜선**(PC를 DHCP로 두면 `172.22.11.X`를 받음) `ssh <사용자>@172.22.11.2`, 또는 **Wi-Fi AP** `VMX-1234`(비밀번호 `password`)에 붙어서 `ssh <사용자>@10.12.34.2`.

무선이나 유선으로 SSH 중이면 `--force-network` 없이는 거부한다(세션이 끊기기 때문; `sudo`가 환경변수를 지워도 부모 프로세스에서 찾는다). **되돌리기:** 로봇의 화면과 키보드에서 `sudo /opt/robot_manager/bin/undo_network.sh`(또는 `install.sh --undo-network`)가 netplan 파일을 복구하고 `systemd-networkd`를 되살린다.
`setupHotspot`·`toggleNetMode`·`setupEthernet`은 접속 중인 SSH 세션을 끊을 변경이어도 거부하지 않는다: 로봇이 3초 뒤 `systemd-run`으로 직접 적용하므로 세션이 끊겨도 변경은 끝까지 된다(`--force`는 필요 없음, 받기만 한다). 끊기면 새 네트워크로 다시 접속한다.
`setupHotspot`/`setupEthernet`의 `--no-activate`는 연결 파일만 쓰고 아무것도 켜지 않는다(`install.sh`가 사용). NetworkManager의 `shared` 모드로 동작하며 비밀번호는 명령줄에 나오지 않는다.
**PC에서만 테스트했다**(NetworkManager를 흉내 낸 단위 테스트 39개 + `install.sh --dry-run`). **로봇에서는 아직 실행하지 않았다.** 로봇의 현재 구성은 `system/network_probe.sh`(읽기 전용)로 확인한다(2026-10-11 개발용 로봇: systemd-networkd + netplan, NetworkManager/hostapd/dnsmasq/iw 없음).

## CPU 최대 성능 (`system/`)

`install.sh`가 `robot_cpu_max.service`를 설치해서 부팅마다 CPU governor를 `performance`로, 최소 주파수를 최대로 올린다(20 ms 루프가 클럭이 올라오길 기다리지 않게). Ubuntu의 `ondemand.service`는 부팅 1분 뒤에 이를 되돌리므로 끈다. `--skip-cpu-max`로 건너뛴다. 오버클럭은 하지 않는다.

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
- `stop_grace_seconds` (선택): 정지 신호들에 주는 전체 시간(초), 양수. 기본 5. 신호마다 균등하게 나눈다.
- `stop_signals` (선택): 정지 신호 순서. `SIGINT`, `SIGTERM`, `SIGHUP`, `SIGQUIT`, `SIGUSR1`, `SIGUSR2` 중에서. 기본 `["SIGINT", "SIGTERM"]`.
- 환경변수 우선순위: 서비스의 환경 < `env` < 일치한 프로필의 `env`. 값은 모두 문자열로 변환된다.
- 프로필은 위에서부터 보고 `kernel_regex`가 처음 일치하는 것 하나만 적용한다.

> **프로필 값은 비어 있다.** 핀 번호 오프셋이 OS/커널에 따라 필요할 수 있다는 것은 사용자 기억일 뿐 버전과 값을 모른다.
> 개발용 VMX(Ubuntu 26.04, 커널 7.0)에서 확인한 것: GPIO sysfs가 꺼져 있어 pigpio 인터럽트가 안 되지만(`PI_BAD_ISR_INIT`) 이것은 오프셋 문제가 아니다.
> 핀 읽기/쓰기에 오프셋이 필요한지는 핀 토글 테스트로 확인한 뒤, 필요하면 근거와 함께 채울 것. 예시 파일의 값은 실제로 일치하지 않는다.

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
