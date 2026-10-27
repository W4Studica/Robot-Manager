/* Tests for gpio_isr_shim.c. The GPIO character device is replaced by pipes: writing the
 * byte '1' or '0' into a gpio's pipe stands for a rising or falling edge. */

#define GPIO_ISR_SHIM_TEST
#include "gpio_isr_shim.c"

#include <assert.h>
#include <signal.h>

static int g_fail;
#define CHECK(cond)                                               \
  do {                                                            \
    if (!(cond)) {                                                \
      fprintf(stderr, "FAIL %s:%d: %s\n", __FILE__, __LINE__, #cond); \
      g_fail++;                                                   \
    }                                                             \
  } while (0)

/* ---- fake backend ------------------------------------------------------------------ */
static int g_pipe_w[PI_MAX_GPIO + 1];
static bool g_request_fails;
static int g_last_edge = -1;

static int fake_request(unsigned gpio, unsigned edge) {
  if (g_request_fails) {
    return -EBUSY;
  }
  g_last_edge = (int)edge;
  int p[2];
  if (pipe2(p, O_NONBLOCK | O_CLOEXEC) != 0) {
    return -errno;
  }
  g_pipe_w[gpio] = p[1];
  return p[0];
}

static int fake_read_event(int fd, int *level) {
  char c;
  ssize_t n = read(fd, &c, 1);
  if (n == 1) {
    *level = c == '1' ? 1 : 0;
    return 0;
  }
  if (n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
    return 1;
  }
  return -1;
}

static void edge(int gpio, int rising) {
  char c = rising ? '1' : '0';
  CHECK(write(g_pipe_w[gpio], &c, 1) == 1);
}

/* ---- callback recorder ------------------------------------------------------------- */
#define MAX_EVENTS 64
static struct {
  int gpio[MAX_EVENTS];
  int level[MAX_EVENTS];
  uint32_t tick[MAX_EVENTS];
  void *ud[MAX_EVENTS];
  int count;
} g_ev;
static pthread_mutex_t g_ev_mutex = PTHREAD_MUTEX_INITIALIZER;

static void record(int gpio, int level, uint32_t tick, void *ud) {
  pthread_mutex_lock(&g_ev_mutex);
  if (g_ev.count < MAX_EVENTS) {
    g_ev.gpio[g_ev.count] = gpio;
    g_ev.level[g_ev.count] = level;
    g_ev.tick[g_ev.count] = tick;
    g_ev.ud[g_ev.count] = ud;
    g_ev.count++;
  }
  pthread_mutex_unlock(&g_ev_mutex);
}
static void cb(int gpio, int level, uint32_t tick) { record(gpio, level, tick, NULL); }
static void cb_other(int gpio, int level, uint32_t tick) { record(gpio + 100, level, tick, NULL); }

static int events(void) {
  pthread_mutex_lock(&g_ev_mutex);
  int n = g_ev.count;
  pthread_mutex_unlock(&g_ev_mutex);
  return n;
}
static void reset_events(void) {
  pthread_mutex_lock(&g_ev_mutex);
  memset(&g_ev, 0, sizeof g_ev);
  pthread_mutex_unlock(&g_ev_mutex);
}
static bool wait_events(int n, int ms) {
  for (int waited = 0; waited < ms; ++waited) {
    if (events() >= n) {
      return true;
    }
    usleep(1000);
  }
  return events() >= n;
}

static void begin(void) {
  shim_test_stop_all();
  reset_events();
  g_request_fails = false;
  g_last_edge = -1;
}

/* ---- tests ------------------------------------------------------------------------- */

static void test_rising_and_falling(void) {
  begin();
  CHECK(gpioSetISRFunc(17, SHIM_EDGE_EITHER, 0, cb) == 0);
  CHECK(g_last_edge == SHIM_EDGE_EITHER);
  edge(17, 1);
  edge(17, 0);
  CHECK(wait_events(2, 2000));
  CHECK(g_ev.gpio[0] == 17 && g_ev.level[0] == 1);
  CHECK(g_ev.gpio[1] == 17 && g_ev.level[1] == 0);
  CHECK(g_ev.tick[1] >= g_ev.tick[0]); /* microseconds, monotonic */
  shim_test_stop_all();
}

static void test_edge_is_passed_to_the_backend(void) {
  begin();
  CHECK(gpioSetISRFunc(5, SHIM_EDGE_RISING, 0, cb) == 0);
  CHECK(g_last_edge == SHIM_EDGE_RISING);
  CHECK(gpioSetISRFunc(6, SHIM_EDGE_FALLING, 0, cb) == 0);
  CHECK(g_last_edge == SHIM_EDGE_FALLING);
  shim_test_stop_all();
}

static void test_timeout_reports_level_2(void) {
  begin();
  CHECK(gpioSetISRFunc(4, SHIM_EDGE_EITHER, 20, cb) == 0);
  CHECK(wait_events(2, 2000)); /* repeats while no edge arrives */
  CHECK(g_ev.level[0] == 2 && g_ev.level[1] == 2);
  shim_test_stop_all();
}

static void test_no_timeout_when_zero(void) {
  begin();
  CHECK(gpioSetISRFunc(4, SHIM_EDGE_EITHER, 0, cb) == 0);
  usleep(100 * 1000);
  CHECK(events() == 0);
  shim_test_stop_all();
}

static void test_userdata(void) {
  begin();
  int marker = 0;
  void (*ex)(int, int, uint32_t, void *) = record;
  CHECK(gpioSetISRFuncEx(9, SHIM_EDGE_EITHER, 0, ex, &marker) == 0);
  edge(9, 1);
  CHECK(wait_events(1, 2000));
  CHECK(g_ev.ud[0] == &marker && g_ev.gpio[0] == 9 && g_ev.level[0] == 1);
  shim_test_stop_all();
}

static void test_cancel(void) {
  begin();
  CHECK(gpioSetISRFunc(12, SHIM_EDGE_EITHER, 0, cb) == 0);
  edge(12, 1);
  CHECK(wait_events(1, 2000));
  int w = g_pipe_w[12];
  CHECK(gpioSetISRFunc(12, SHIM_EDGE_EITHER, 0, NULL) == 0);
  /* The read end is closed, so writing now fails; either way no new callback may arrive. */
  signal(SIGPIPE, SIG_IGN);
  (void)!write(w, "1", 1);
  usleep(100 * 1000);
  CHECK(events() == 1);
  /* Cancelling something that is not set is fine. */
  CHECK(gpioSetISRFunc(13, SHIM_EDGE_EITHER, 0, NULL) == 0);
}

static void test_replace_switches_the_callback(void) {
  begin();
  CHECK(gpioSetISRFunc(20, SHIM_EDGE_EITHER, 0, cb) == 0);
  CHECK(gpioSetISRFunc(20, SHIM_EDGE_EITHER, 0, cb_other) == 0);
  edge(20, 1);
  CHECK(wait_events(1, 2000));
  CHECK(g_ev.gpio[0] == 120); /* cb_other adds 100 */
  usleep(50 * 1000);
  CHECK(events() == 1);
  shim_test_stop_all();
}

static void test_independent_gpios(void) {
  begin();
  CHECK(gpioSetISRFunc(1, SHIM_EDGE_EITHER, 0, cb) == 0);
  CHECK(gpioSetISRFunc(2, SHIM_EDGE_EITHER, 0, cb) == 0);
  edge(2, 1);
  CHECK(wait_events(1, 2000));
  CHECK(g_ev.gpio[0] == 2);
  usleep(50 * 1000);
  CHECK(events() == 1);
  shim_test_stop_all();
}

static void test_argument_errors(void) {
  begin();
  CHECK(gpioSetISRFunc(54, SHIM_EDGE_EITHER, 0, cb) == PI_BAD_GPIO);
  CHECK(gpioSetISRFunc(1000, SHIM_EDGE_EITHER, 0, cb) == PI_BAD_GPIO);
  CHECK(gpioSetISRFunc(3, 3, 0, cb) == PI_BAD_EDGE);
  CHECK(gpioSetISRFunc(53, SHIM_EDGE_EITHER, 0, cb) == 0); /* 53 is the highest valid GPIO */
  shim_test_stop_all();
}

static void test_request_failure_is_bad_isr_init(void) {
  begin();
  g_request_fails = true;
  CHECK(gpioSetISRFunc(7, SHIM_EDGE_EITHER, 0, cb) == PI_BAD_ISR_INIT);
  /* A failed set leaves nothing behind: a later good request works. */
  g_request_fails = false;
  CHECK(gpioSetISRFunc(7, SHIM_EDGE_EITHER, 0, cb) == 0);
  shim_test_stop_all();
}

static void test_terminate_stops_everything(void) {
  begin();
  CHECK(gpioSetISRFunc(30, SHIM_EDGE_EITHER, 0, cb) == 0);
  CHECK(gpioSetISRFunc(31, SHIM_EDGE_EITHER, 0, cb) == 0);
  int w = g_pipe_w[30];
  gpioTerminate();
  signal(SIGPIPE, SIG_IGN);
  (void)!write(w, "1", 1);
  usleep(100 * 1000);
  CHECK(events() == 0);
}

static void test_burst_of_events_is_not_lost(void) {
  begin();
  CHECK(gpioSetISRFunc(40, SHIM_EDGE_EITHER, 0, cb) == 0);
  for (int i = 0; i < 40; ++i) {
    edge(40, i % 2);
  }
  CHECK(wait_events(40, 3000));
  for (int i = 0; i < 40 && i < events(); ++i) {
    CHECK(g_ev.level[i] == i % 2); /* in order */
  }
  shim_test_stop_all();
}

static atomic_int self_cancel_ok;
static void cb_cancel_self(int gpio, int level, uint32_t tick) {
  (void)level;
  (void)tick;
  /* Cancelling from inside the callback must not deadlock. */
  CHECK(gpioSetISRFunc((unsigned)gpio, SHIM_EDGE_EITHER, 0, NULL) == 0);
  atomic_store(&self_cancel_ok, 1);
}

static void test_cancel_from_inside_the_callback(void) {
  begin();
  atomic_store(&self_cancel_ok, 0);
  CHECK(gpioSetISRFunc(25, SHIM_EDGE_EITHER, 0, cb_cancel_self) == 0);
  edge(25, 1);
  for (int i = 0; i < 2000 && !atomic_load(&self_cancel_ok); ++i) {
    usleep(1000);
  }
  CHECK(atomic_load(&self_cancel_ok) == 1);
  shim_test_stop_all();
}

/* The HAL is loaded with dlopen(RTLD_LOCAL), like halsim_studica's Studica plugin does. dlsym(RTLD_NEXT)
 * cannot see it; the shim must still forward gpioTerminate() and gpioTick() to it. */
static void test_forwards_to_a_locally_loaded_hal(void) {
  begin();
  const char *name = "./libfake_hal.so";
  void *hal = dlopen(name, RTLD_NOW | RTLD_LOCAL);
  CHECK(hal != NULL);
  if (hal == NULL) {
    fprintf(stderr, "dlopen: %s\n", dlerror());
    return;
  }
  int (*calls)(void) = (int (*)(void))dlsym(hal, "fake_hal_terminate_calls");
  CHECK(calls != NULL);
  CHECK(dlsym(RTLD_NEXT, "gpioTerminate") == NULL); /* precondition: RTLD_NEXT really cannot see it */

  setenv("VMX_GPIO_SHIM_REAL_LIB", name, 1);
  CHECK(gpioSetISRFunc(14, SHIM_EDGE_EITHER, 0, cb) == 0);
  int before = calls();
  gpioTerminate();
  CHECK(calls() == before + 1);     /* the real gpioTerminate ran */
  signal(SIGPIPE, SIG_IGN);         /* the read end is closed now, so this write is expected to fail */
  (void)!write(g_pipe_w[14], "1", 1);
  usleep(100 * 1000);
  CHECK(events() == 0);             /* and our ISR threads were stopped first */

  /* gpioTick() also comes from the HAL: the callback tick must be its value. */
  CHECK(gpioSetISRFunc(15, SHIM_EDGE_EITHER, 0, cb) == 0);
  edge(15, 1);
  CHECK(wait_events(1, 2000));
  CHECK(g_ev.tick[0] == 424242u);
  shim_test_stop_all();

  unsetenv("VMX_GPIO_SHIM_REAL_LIB");
  dlclose(hal);
}

static void test_missing_real_library_is_not_fatal(void) {
  begin();
  setenv("VMX_GPIO_SHIM_REAL_LIB", "libdoes_not_exist_anywhere.so", 1);
  gpioTerminate(); /* prints a warning, must not crash */
  unsetenv("VMX_GPIO_SHIM_REAL_LIB");
  CHECK(1);
}

int main(void) {
  setenv("VMX_GPIO_SHIM_FORCE", "1", 1); /* do not depend on /sys/class/gpio of the test machine */
  setenv("VMX_GPIO_SHIM_QUIET", "1", 1); /* the tests have no real HAL; do not print the "not found" warning */
  shim_test_set_backend((backend_t){fake_request, fake_read_event});

  test_rising_and_falling();
  test_edge_is_passed_to_the_backend();
  test_timeout_reports_level_2();
  test_no_timeout_when_zero();
  test_userdata();
  test_cancel();
  test_replace_switches_the_callback();
  test_independent_gpios();
  test_argument_errors();
  test_request_failure_is_bad_isr_init();
  test_terminate_stops_everything();
  test_burst_of_events_is_not_lost();
  test_cancel_from_inside_the_callback();
  test_forwards_to_a_locally_loaded_hal();
  test_missing_real_library_is_not_fatal();

  if (g_fail) {
    fprintf(stderr, "%d check(s) failed\n", g_fail);
    return 1;
  }
  puts("all gpio_isr_shim tests passed");
  return 0;
}
