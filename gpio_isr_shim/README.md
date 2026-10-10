# gpio_isr_shim

`LD_PRELOAD` 라이브러리. Studica VMX HAL(`libvmxpi_hal_cpp.so`) 안의 pigpio가 GPIO 인터럽트를 sysfs GPIO(`/sys/class/gpio`)로 구현하기 때문에,
**sysfs GPIO가 없는 커널**에서는 HAL을 열 때 이런 메시지가 나온다.

```
RPI GPIO Interrupt Enable:  PI_BAD_ISR_INIT.
```

개발용 VMX(Ubuntu 26.04, 커널 `7.0.0-1020-raspi`)는 `# CONFIG_GPIO_SYSFS is not set`이고 `/sys/class/gpio`가 없다. `strace`로 HAL이
`/sys/class/gpio/export`를 3번 열려다 `ENOENT`로 실패하는 것을 확인했다(메시지 3번과 일치).

## 무엇을 하는가

HAL은 `gpioSetISRFunc` / `gpioSetISRFuncEx`를 PLT로 호출하고(`R_AARCH64_JUMP_SLOT`) 이 심볼들은 라이브러리가 내보낸다(`nm -D`로 확인).
그래서 이 shim이 두 함수와 `gpioTerminate`를 대체한다.

- 에지 이벤트는 **GPIO 문자 장치**(`/dev/gpiochip0`, uAPI v2)로 받는다.
- 콜백 규약은 `pigpio.h`와 같다: `(gpio, level, tick)`. `level` 0=하강, 1=상승, 2=타임아웃(`timeout` ms 동안 에지 없음), `tick`은 `gpioTick()`(마이크로초).
- GPIO마다 스레드 하나. `f == NULL`이면 취소. 같은 GPIO에 다시 설정하면 교체.
- 반환값: 성공 0, GPIO 범위 밖 `PI_BAD_GPIO`(-3), 에지 값 오류 `PI_BAD_EDGE`(-122), 문자 장치를 못 열거나 줄 요청이 실패하면 `PI_BAD_ISR_INIT`(-123).
- `/sys/class/gpio`가 **있으면** 원래 함수로 넘긴다(다른 환경에 영향 없음). `VMX_GPIO_SHIM_FORCE=1`이면 항상 이 구현을 쓴다.
- `gpioTerminate`에서 ISR 스레드를 먼저 멈춘 뒤 원래 함수를 부른다.
- `VMX_GPIO_SHIM_DEBUG=1`이면 모든 호출을 stderr에 기록한다(HAL이 어떤 GPIO를 어떤 에지로 요청하는지 볼 수 있다).

## 빌드, 테스트

```bash
make        # libvmx_gpio_isr_shim.so
make test   # GPIO 하드웨어 없이 돈다: 문자 장치 자리에 파이프를 끼워서 콜백 규약을 검사한다
```

테스트 13개: 상승/하강, 에지 인자 전달, 타임아웃(level 2), `timeout=0`이면 타임아웃 없음, `userdata`, 취소, 교체, GPIO 독립성,
인자 오류 코드, 요청 실패 후 복구, `gpioTerminate`, 이벤트 연속 도착 시 순서와 누락, 콜백 안에서 자기 자신을 취소(교착 방지).
ThreadSanitizer(0건), AddressSanitizer, UBSan으로도 통과시켰다(TSan이 종료 플래그의 데이터 레이스를 찾아서 `atomic_int`로 고쳤다).

## 사용

`robot_manager`의 `config.json`에서 로봇 프로그램의 환경으로 준다(`install.sh`가 `/opt/robot_manager/lib/`에 설치한다).

```json
{ "env": { "LD_PRELOAD": "/opt/robot_manager/lib/libvmx_gpio_isr_shim.so" } }
```

수동으로 시험할 때는 `sudo`가 `LD_PRELOAD`를 지우므로 `env`를 거친다:

```bash
sudo env VMX_GPIO_SHIM_DEBUG=1 LD_PRELOAD=/opt/robot_manager/lib/libvmx_gpio_isr_shim.so ./vmx_n 1
```

## 로봇에서 확인한 것 (2026-10-10, Ubuntu 26.04 / 커널 7.0 / Pi 4)

`sudo env VMX_GPIO_SHIM_DEBUG=1 LD_PRELOAD=... ./vmx_n 1` 결과:

- `RPI GPIO Interrupt Enable:  PI_BAD_ISR_INIT.` 3줄이 **사라졌다.**
- HAL이 요청한 인터럽트: **GPIO 13, 6, 12, 모두 `edge=1`(하강), `timeout=0`.**
- 요청이 전부 성공했고(`cannot request` 없음), GPIO 12에서 **하강 에지가 약 20 ms 간격(50 Hz)으로 계속 도착**한다. `VMXPi(true, 50)`의 50 Hz와 일치하므로 VMX 보드의 "데이터 준비됨" 신호로 보인다.
- 보드 통신(`Established communication with VMX board ...`), navX 인식, `IsOpen: 1`은 그대로.
- **끝까지 실행(디버그 로그 없이):** 정상 종료(`VMX HAL:  pigpio library closed.`, 종료 코드 0). 쓰기 19회에 `Write CRC Mismatches: 0`/`Write Failures: 0`, 읽기 9051회에 실패 0. `gpioTerminate`를 가로채서 ISR 스레드를 먼저 멈추는 것도 문제없이 동작했다.
- 아직 확인 못 한 것: GPIO 6/13에서 에지가 오는지(이 실행에서는 12만 도착), 이 인터럽트가 HAL의 어떤 기능에 쓰이는지, shim 유무에 따른 동작 차이(이전에도 통신은 정상이었다).

## 알려진 한계

- PC에서는 콜백 규약만 검증했고, 실제 에지 도착은 위처럼 로봇에서 확인했다(위의 "아직 확인 못 한 것" 제외).
- 커널에 줄을 요청하면 그 핀이 GPIO 입력으로 잡힌다. HAL이 같은 핀을 다른 용도로 쓰면 충돌(`EBUSY`)하거나 핀 기능이 바뀔 수 있다.
  충돌하면 `PI_BAD_ISR_INIT`이 그대로 나오므로(전과 같음) 나빠지지는 않는다. `VMX_GPIO_SHIM_DEBUG=1`로 원인(errno)을 볼 수 있다.
- HAL이 인터럽트로 무엇을 하는지(어떤 기능이 이 인터럽트에 의존하는지)는 아직 모른다. HAL의 `comm ready` 핸드셰이크는 이미 문자 장치로 동작한다(SPI 통신은 정상).
- 콜백은 GPIO별 스레드에서 실행된다. 콜백 안에서 `gpioSetISRFunc`로 자기 자신을 취소하는 것은 지원한다(다른 GPIO를 취소/교체하면서 서로를 기다리는 경우는 지원하지 않는다).
- 리눅스 전용.
