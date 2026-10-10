/*
 * gpio_isr_shim - gpioSetISRFunc()/gpioSetISRFuncEx() on top of the GPIO character device.
 *
 * Why: the pigpio inside libvmxpi_hal_cpp.so implements GPIO interrupts with sysfs GPIO
 * (/sys/class/gpio). Kernels built without CONFIG_GPIO_SYSFS (for example Ubuntu 26.04's
 * 7.0 raspi kernel) have no such directory, so the HAL prints
 *     RPI GPIO Interrupt Enable:  PI_BAD_ISR_INIT.
 * and RPi GPIO interrupts do not work. The HAL calls gpioSetISRFunc* through the PLT, so
 * an LD_PRELOAD library can replace them:
 *
 *     LD_PRELOAD=/opt/robot_manager/lib/libvmx_gpio_isr_shim.so ./robot
 *
 * Behaviour (pigpio.h): the callback gets (gpio, level, tick). level is 0 for a falling
 * edge, 1 for a rising edge and 2 (PI_TIMEOUT) when `timeout` ms pass without an edge.
 * tick is microseconds from gpioTick(). One thread per GPIO. f == NULL cancels.
 *
 * If /sys/class/gpio exists the original functions are used (set VMX_GPIO_SHIM_FORCE=1 to
 * use this implementation anyway). VMX_GPIO_SHIM_DEBUG=1 logs every call to stderr.
 *
 * Only built for Linux. Not verified on hardware: see README.md in this directory.
 */

#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <linux/gpio.h>
#include <poll.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/eventfd.h>
#include <sys/ioctl.h>
#include <time.h>
#include <unistd.h>

/* pigpio.h */
#define PI_BAD_GPIO (-3)
#define PI_BAD_EDGE (-122)
#define PI_BAD_ISR_INIT (-123)
#define PI_MAX_GPIO 53
#define SHIM_EDGE_RISING 0
#define SHIM_EDGE_FALLING 1
#define SHIM_EDGE_EITHER 2

typedef void (*isr_func_t)(int gpio, int level, uint32_t tick);
typedef void (*isr_func_ex_t)(int gpio, int level, uint32_t tick, void *userdata);

/* Where edge events come from. The default is the GPIO character device; tests replace it. */
typedef struct {
  /* Returns a pollable fd that reports edges on `gpio`, or -errno. */
  int (*request)(unsigned gpio, unsigned edge);
  /* Reads one event: 0 = got one (*level set), 1 = none available, -1 = error. */
  int (*read_event)(int fd, int *level);
} backend_t;

typedef struct {
  bool active;
  int gpio;
  int fd;
  int wake_fd;
  int timeout_ms;
  isr_func_t f;
  isr_func_ex_t fex;
  void *userdata;
  pthread_t thread;
  atomic_int stop; /* written by the thread that cancels, read by the ISR thread */
} isr_slot_t;

static isr_slot_t g_slots[PI_MAX_GPIO + 1];
static pthread_mutex_t g_config_mutex = PTHREAD_MUTEX_INITIALIZER;
static bool g_debug;

#define DEBUG(...)                         \
  do {                                     \
    if (g_debug) {                         \
      fprintf(stderr, "gpio_isr_shim: ");  \
      fprintf(stderr, __VA_ARGS__);        \
      fputc('\n', stderr);                 \
    }                                      \
  } while (0)

/* ---------------------------------------------------------------- GPIO character device */

static int chardev_request(unsigned gpio, unsigned edge) {
  int chip = open("/dev/gpiochip0", O_RDONLY | O_CLOEXEC);
  if (chip < 0) {
    return -errno;
  }
  struct gpio_v2_line_request req;
  memset(&req, 0, sizeof req);
  req.offsets[0] = gpio;
  req.num_lines = 1;
  strncpy(req.consumer, "vmx-gpio-isr-shim", sizeof req.consumer - 1);
  req.config.flags = GPIO_V2_LINE_FLAG_INPUT;
  if (edge == SHIM_EDGE_RISING || edge == SHIM_EDGE_EITHER) {
    req.config.flags |= GPIO_V2_LINE_FLAG_EDGE_RISING;
  }
  if (edge == SHIM_EDGE_FALLING || edge == SHIM_EDGE_EITHER) {
    req.config.flags |= GPIO_V2_LINE_FLAG_EDGE_FALLING;
  }
  int rc = ioctl(chip, GPIO_V2_GET_LINE_IOCTL, &req);
  int err = errno;
  close(chip);
  if (rc < 0) {
    return -err;
  }
  int flags = fcntl(req.fd, F_GETFL);
  if (flags < 0 || fcntl(req.fd, F_SETFL, flags | O_NONBLOCK) < 0) {
    err = errno;
    close(req.fd);
    return -err;
  }
  return req.fd;
}

static int chardev_read_event(int fd, int *level) {
  struct gpio_v2_line_event ev;
  ssize_t n = read(fd, &ev, sizeof ev);
  if (n == (ssize_t)sizeof ev) {
    *level = ev.id == GPIO_V2_LINE_EVENT_RISING_EDGE ? 1 : 0;
    return 0;
  }
  if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
    return 1;
  }
  return -1;
}

static backend_t g_backend = {chardev_request, chardev_read_event};

/* ---------------------------------------------------------------- helpers */

static uint32_t shim_tick(void) {
  static uint32_t (*real_tick)(void);
  static bool looked_up;
  if (!looked_up) {
    real_tick = (uint32_t(*)(void))dlsym(RTLD_NEXT, "gpioTick");
    looked_up = true;
  }
  if (real_tick) {
    return real_tick();
  }
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return (uint32_t)((uint64_t)ts.tv_sec * 1000000u + (uint64_t)ts.tv_nsec / 1000u);
}

static bool use_shim(void) {
  const char *force = getenv("VMX_GPIO_SHIM_FORCE");
  if (force && *force == '1') {
    return true;
  }
  return access("/sys/class/gpio/export", F_OK) != 0;
}

static void call_isr(isr_slot_t *s, int level) {
  uint32_t tick = shim_tick();
  DEBUG("gpio %d level %d tick %u", s->gpio, level, tick);
  if (s->fex) {
    s->fex(s->gpio, level, tick, s->userdata);
  } else if (s->f) {
    s->f(s->gpio, level, tick);
  }
}

static void *isr_thread(void *arg) {
  isr_slot_t *s = arg;
  struct pollfd pfd[2];
  pfd[0].fd = s->fd;
  pfd[0].events = POLLIN;
  pfd[1].fd = s->wake_fd;
  pfd[1].events = POLLIN;

  while (!atomic_load(&s->stop)) {
    pfd[0].revents = pfd[1].revents = 0;
    int r = poll(pfd, 2, s->timeout_ms > 0 ? s->timeout_ms : -1);
    if (r < 0) {
      if (errno == EINTR) {
        continue;
      }
      break;
    }
    if (pfd[1].revents || atomic_load(&s->stop)) {
      break;
    }
    if (r == 0) {
      call_isr(s, 2); /* PI_TIMEOUT */
      continue;
    }
    if (pfd[0].revents & POLLIN) {
      for (;;) {
        int level = 0;
        int rc = g_backend.read_event(s->fd, &level);
        if (rc != 0) {
          break;
        }
        call_isr(s, level);
        if (atomic_load(&s->stop)) {
          break;
        }
      }
    } else if (pfd[0].revents & (POLLERR | POLLHUP | POLLNVAL)) {
      break;
    }
  }
  return NULL;
}

/* Caller holds g_config_mutex. Stops the ISR on `gpio` if there is one. */
static void stop_locked(int gpio) {
  isr_slot_t *s = &g_slots[gpio];
  if (!s->active) {
    return;
  }
  atomic_store(&s->stop, 1);
  uint64_t one = 1;
  if (write(s->wake_fd, &one, sizeof one) < 0) {
    /* the thread also checks `stop` on every pass */
  }
  if (pthread_equal(pthread_self(), s->thread)) {
    pthread_detach(s->thread); /* cancelled from its own callback: cannot join itself */
  } else {
    pthread_join(s->thread, NULL);
  }
  close(s->fd);
  close(s->wake_fd);
  memset(s, 0, sizeof *s);
}

static int set_isr(unsigned gpio, unsigned edge, int timeout_ms, isr_func_t f, isr_func_ex_t fex,
                   void *userdata) {
  DEBUG("set_isr gpio=%u edge=%u timeout=%d %s", gpio, edge, timeout_ms,
        (f || fex) ? "enable" : "cancel");
  if (gpio > PI_MAX_GPIO) {
    return PI_BAD_GPIO;
  }
  if (edge > SHIM_EDGE_EITHER) {
    return PI_BAD_EDGE;
  }

  pthread_mutex_lock(&g_config_mutex);
  stop_locked((int)gpio);
  if (f == NULL && fex == NULL) {
    pthread_mutex_unlock(&g_config_mutex);
    return 0;
  }

  int fd = g_backend.request(gpio, edge);
  if (fd < 0) {
    DEBUG("cannot request gpio %u: %s", gpio, strerror(-fd));
    pthread_mutex_unlock(&g_config_mutex);
    return PI_BAD_ISR_INIT;
  }
  int wake_fd = eventfd(0, EFD_CLOEXEC);
  if (wake_fd < 0) {
    close(fd);
    pthread_mutex_unlock(&g_config_mutex);
    return PI_BAD_ISR_INIT;
  }

  isr_slot_t *s = &g_slots[gpio];
  memset(s, 0, sizeof *s);
  s->active = true;
  s->gpio = (int)gpio;
  s->fd = fd;
  s->wake_fd = wake_fd;
  s->timeout_ms = timeout_ms;
  s->f = f;
  s->fex = fex;
  s->userdata = userdata;
  if (pthread_create(&s->thread, NULL, isr_thread, s) != 0) {
    close(fd);
    close(wake_fd);
    memset(s, 0, sizeof *s);
    pthread_mutex_unlock(&g_config_mutex);
    return PI_BAD_ISR_INIT;
  }
  pthread_mutex_unlock(&g_config_mutex);
  return 0;
}

static void stop_all(void) {
  pthread_mutex_lock(&g_config_mutex);
  for (int i = 0; i <= PI_MAX_GPIO; ++i) {
    stop_locked(i);
  }
  pthread_mutex_unlock(&g_config_mutex);
}

__attribute__((constructor)) static void shim_init(void) {
  const char *d = getenv("VMX_GPIO_SHIM_DEBUG");
  g_debug = d && *d == '1';
}

/* ---------------------------------------------------------------- interposed pigpio API */

#ifndef GPIO_ISR_SHIM_TEST
#define SHIM_EXPORT __attribute__((visibility("default")))
#else
#define SHIM_EXPORT static
#endif

SHIM_EXPORT int gpioSetISRFunc(unsigned gpio, unsigned edge, int timeout, isr_func_t f) {
  if (!use_shim()) {
    int (*real)(unsigned, unsigned, int, isr_func_t) =
        (int (*)(unsigned, unsigned, int, isr_func_t))dlsym(RTLD_NEXT, "gpioSetISRFunc");
    return real ? real(gpio, edge, timeout, f) : PI_BAD_ISR_INIT;
  }
  return set_isr(gpio, edge, timeout, f, NULL, NULL);
}

SHIM_EXPORT int gpioSetISRFuncEx(unsigned gpio, unsigned edge, int timeout, isr_func_ex_t f,
                                 void *userdata) {
  if (!use_shim()) {
    int (*real)(unsigned, unsigned, int, isr_func_ex_t, void *) =
        (int (*)(unsigned, unsigned, int, isr_func_ex_t, void *))dlsym(RTLD_NEXT, "gpioSetISRFuncEx");
    return real ? real(gpio, edge, timeout, f, userdata) : PI_BAD_ISR_INIT;
  }
  return set_isr(gpio, edge, timeout, NULL, f, userdata);
}

/* Stop our ISR threads before pigpio tears itself down, so no callback runs into freed state. */
SHIM_EXPORT void gpioTerminate(void) {
  stop_all();
  void (*real)(void) = (void (*)(void))dlsym(RTLD_NEXT, "gpioTerminate");
  if (real) {
    real();
  }
}

#ifdef GPIO_ISR_SHIM_TEST
/* Test hooks. */
static void shim_test_set_backend(backend_t b) { g_backend = b; }
static void shim_test_stop_all(void) { stop_all(); }
#endif
