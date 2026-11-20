# JunoOS

JunoOS is an operating system designed from the ground up in Rust, living
entirely in the Rust ecosystem. The premise: mainstream operating systems
carry decades of accumulated design constraints — C ABIs, unsafe-by-default
interfaces, invariants enforced by convention and runtime checks because
their languages could not express them. Rust was created on the principle of
keeping the lessons of previous language generations while discarding their
accumulated compromises; JunoOS applies that same principle to the operating
system. Concretely: ownership and the type system move OS invariants
to compile time wherever the language can express them — forging a pin
handle, configuring the same pin twice, and writing an error branch for an
operation that cannot fail are all build errors. The invariants that are
inherently per-boot facts — has the board been claimed yet, has this driver
been brought up yet — cannot be compile-time, and are instead enforced by
runtime constructors that succeed at most once. The
[API section](#the-api-invariants-at-compile-time) below shows exactly which
is which, in the code that exists today.

It is also built for the joy of building it: the author is a professional
engineer doing this because writing code is fun.

## The first target: Raspberry Pi Pico 2 (RP2350)

The first hardware target is the Raspberry Pi Pico 2 board and its RP2350
microcontroller, brought up fully bare metal: no operating system and no
vendor library underneath the code. This workspace contains every piece the
chip needs to boot and blink, all written by hand and traced to datasheet
citations:

- the boot metadata the chip's **bootrom** looks for (the bootrom is the
  fixed program in the chip's ROM that runs first after power-on, finds a
  valid image in flash, and starts it);
- the **vector table**, the array at a fixed address whose entries the CPU
  reads directly: the initial stack pointer, then the addresses of the
  functions to run on reset and on each interrupt;
- the **reset handler**, the function named in that table that runs before
  anything else of ours;
- the **linker script**, the file that tells the linker at which address in
  the chip's memory map to place each part of the program;
- a **GPIO** driver (general-purpose input/output — a chip pin the
  program can drive high or low, or read the level of);
- and drivers for the **clock tree** (crystal oscillator, PLLs, 150 MHz
  system clock, 48 MHz USB clock), the microsecond **timer**, the
  **watchdog**, a polled **USB serial port** (a CDC-ACM device, which
  desktop operating systems open as a serial port without a driver install),
  **PWM** square-wave outputs (a fixed-frequency tone on a pin, switched
  on and off), the **ADC** (analog-to-digital converter) sampling one
  analog pin continuously into memory through a **DMA** channel, the two
  **UARTs** (polled, or with their FIFOs serviced by DMA), the two **I2C**
  and two **SPI** bus controllers, and **starting the second core** with
  its own stack.

The boot pieces and the GPIO driver are taught from scratch in this
repository's [bare-metal tutorial](docs/tutorials/rp2350_baremetal/index.md). No external
embedded crates (`cortex-m`, `cortex-m-rt`, `rp-hal`, `embassy`, …) are
used. The one crate from outside this repository is
[`heapless`](https://docs.rs/heapless/0.9.3/heapless/), the fixed-capacity
queues and vectors `api`'s software bus is built on; it and its own small
dependencies are built with every application, alongside `api`.

## Repository layout

The code is a Cargo **workspace** (one repository containing several Rust
packages, called *crates*, built together) with two library crates. Neither
is a program you flash: an application is a separate binary crate, generated
from `templates/`, that depends on both.

```
junoos/
├── api/                 portable traits, ownership types and the app framework (library)
├── firmware/pico2/      RP2350 runtime + drivers (library)
├── templates/           cargo-generate templates: a Pico 2 application, and an app module
├── docs/                datasheets and manuals, their text extractions, and the bare-metal tutorial
├── cargo-generate.toml  marks the repository as a cargo-generate template source
└── .cargo/config.toml   the `cargo pico2` build alias
```

**`api/`** is the portability boundary. It defines the traits (Rust
interfaces) that application code programs against, one module per
subject:

- `common` — `ErrorType`, `Read`, `Write`, and a fixed-capacity byte ring
  (`common::ring::ByteRing`);
- `device` — the `PinHandle` and `DeviceHandle` ownership types and the
  `define_board!` macro described in the next section;
- `gpio` — `Gpio`, `GpioPinIn`, `GpioPinOut`, `EdgeLatch` and the `Pull`
  enum;
- `pwm`, `adc`, `watchdog`, `i2c`, `spi` — the traits for each peripheral
  kind (for `pwm`, a factory trait and a per-pin trait); a UART needs none of its own, being a byte stream
  through `Read<u8>` and `Write<u8>`;
- `error` — `Error` and `ErrorKind`, so code can tell what *sort* of
  failure an error is without knowing its type;
- `app`, `iload`, `sch`, `sb`, `time` — the application framework,
  described [below](#applications-scheduling-and-messaging).

Its `usb` module holds the chip-independent half of the USB
serial port — descriptors, the endpoint-0 request state machine, CDC-ACM
line coding — unit-tested on the host. Its `multicore`
module is the bootrom's handshake for starting the second core, as a state
machine that never waits, tested on the host against a model of that core.
Its `time` module tags every time with the clock it came from, and has a
`DisciplinedClock`: a free-running counter read as mission elapsed time and
steered to a reference such as GPS UTC, shareable between cores without
locks, with exactly one `Discipliner` allowed to steer it.
The crate contains no register addresses and no dependency on any other
crate in the workspace (its one dependency is `heapless`), so it compiles
for your host machine as well as for the microcontroller, which is what
makes logic written against it testable without hardware.

**`firmware/pico2/`** is a **library** crate (it has `src/lib.rs` and no
`main.rs`) holding everything that must exist before and around user code:

- `BOOT_INFO`, the RP2350 `IMAGE_DEF` metadata block the bootrom requires;
- the Cortex-M vector table and the `OnReset` reset handler, which does the
  work C startup code normally hides: enable the FPU (the floating-point
  unit, powered off at reset), set `VTOR` (the CPU register that holds the
  vector table's address), copy `.data` (the linker section holding the
  initial values of initialized global variables) from flash to RAM, and
  zero `.bss` (the section for zero-initialized globals);
- the `entry!` macro, which an application uses to declare its `main` — the
  macro emits the `__junoos_main` symbol the reset handler calls, and
  type-checks `main` against `fn() -> !` at compile time (`!` is the type of
  a function that never returns; on bare metal there is nothing to return
  to, so a `main` that could fall off the end is a type error); it can also
  name a *safe state*, a function the fault handlers call first, so that a
  fault puts the application's outputs back to their safe level before the
  program stops;
- `common/board.rs`, which instantiates `api`'s `define_board!` macro as the
  `Rp2350` board type with the Pico 2's pin map and a device handle for
  each driver below except `uart_dma`, which is converted from a `uart`;
- a GPIO driver (`Rp2350Gpio`, producing `Rp2350GpioIn` / `Rp2350GpioOut`
  pins) implementing the `api` traits, inputs with their hardware edge
  latch (`EdgeLatch`) for catching short pulses by polling;
- `clocks` (`Rp2350Clocks`): crystal, both PLLs, and the `clk_ref`,
  `clk_sys`, `clk_peri` and `clk_usb` generators (`clk_adc` is started by
  `adc`);
- `timer` (`Rp2350Timer`): the 1 µs tick and the 64-bit `TIMER0` counter,
  with busy-wait delays, never paused by a debugger;
- `systick` (`Rp2350SysTick`): the core's SysTick counter run free on the
  processor clock, a second witness of elapsed time apart from the 1 µs
  tick;
- `watchdog` (`Rp2350Watchdog`): start, feed, forced reset, the reason for
  the last reset, and the two scratch registers a watchdog reset keeps for
  the application;
- `usb` (`Rp2350Usb`): a polled USB CDC-ACM device with non-blocking
  `read`/`write` and a link-lost indicator;
- `pwm` (`Rp2350Pwm`): 50 % square waves of 100 Hz–20 kHz on any GPIO,
  switched on and off at whole cycles, off when dropped, and all off at once
  from a panic handler;
- `dma` (`Rp2350Dma`): the DMA block's 16 channels as owned handles, for a
  driver to take one;
- `adc` (`Rp2350Adc`, `Rp2350AdcStream`): one of GPIO26–29 sampled free
  running at 1–480 kHz, a DMA channel moving each sample into a 16 kB ring
  in RAM, read at the application's own pace with overruns reported;
- `multicore` (`Rp2350Core1`, `Fifo`, `Core1Stack`): starting core 1 on a
  stack of its own with a stack limit that faults instead of overflowing,
  the FIFOs between the cores read and written without waiting, and which
  core is running;
- `uart` (`Rp2350Uart`): the two PL011 UARTs, polled through their 32-byte
  FIFOs, never blocking;
- `uart_dma` (`Rp2350UartDma`, `UartRxRing`, `UartTxRing`): a UART
  converted to use two DMA channels, one filling a 4 kB receive ring and
  one draining a 1 kB transmit ring, so bytes keep moving between polls;
- `i2c` (`Rp2350I2c`): the two I2C controllers, polled, 7-bit addresses,
  10 kHz–1 MHz;
- `spi` (`Rp2350Spi`): the two SPI controllers, polled, 8-bit words in the
  four Motorola modes, with chip-select left to an ordinary GPIO output;
- `common/pinmux.rs` (internal to the crate): which pins can carry which
  UART, I2C and SPI signal, checked at compile time, and the sequence that
  connects a pin to its peripheral;
- `link.ld`, the linker script that places every section in the RP2350's
  memory map, and `build.rs`, which tells Cargo where to find it.

**`templates/`** holds the [cargo-generate](https://github.com/cargo-generate/cargo-generate)
templates. `templates/pico2` generates a standalone Pico 2 application — a
blinky that toggles the on-board LED — with its own `.cargo/config.toml`
(target triple, linker flags, and `picotool` as the `cargo run` runner)
and `api`/`pico2` as git dependencies. Its `main` takes the `Rp2350` board
singleton (a value the program can obtain exactly once — the next section
shows the mechanism), constructs the `Rp2350Gpio` driver, passes the
board's `led` pin handle to that driver to receive a configured output
pin, and drives it through the portable `Write<bool>` trait.
`templates/app` generates one application module (an `api::app::App` with
its configuration table) inside an existing crate.
[`templates/README.md`](templates/README.md) has the commands.

## Applications, scheduling and messaging

The rest of `api` is a small flight-software framework, the Rust form of
LibJuno's application, scheduler and software-bus APIs. Each module's
rustdoc gives the detail; those of `app`, `sch`, `sb` and `time` also
give the mapping to LibJuno.

- `app` — the `App` trait a scheduler drives (`on_process`, `on_exit`),
  and `Health`, the record every app keeps of its runs and reports as
  `HealthTlm`.
- `iload` — configuration tables compiled into the image: every number an
  app is configured with lives in its `table.rs`, checked at compile time
  with `assert_ok`, and `Slot` is the app's place in the schedule.
- `sch` — the `Scheduler` trait and `TdmSch`, a time-division-multiplexed
  cyclic executive: fixed minor frames, a table of which apps run in each,
  and overruns and app failures logged and counted rather than stopping
  the schedule. A schedule is built with `TdmSchBuilder` and cannot change
  once it runs.
- `sb` — the software bus: apps publish messages by id (`Mid`) through a
  broker (`ArrayBroker`), and each subscriber drains its own fixed-size
  `Pipe`. A broker belongs to one thread of execution; `IpcApp` carries
  messages between two of them, such as the two cores, over lock-free
  queues.
- `time` — `Time<C>`, a time tagged with the clock `C` it came from, so
  mixing clocks is a compile error; the `Clock` and `Sleep` traits; and
  `DisciplinedClock`.

## The API: invariants at compile time

This is the part of the mission that already exists in code, so it is worth
stating precisely. Three invariants that embedded C enforces by convention
are enforced here by the type system:

**A pin you hold exists, and nobody else holds it.**
`api::device::PinHandle<const N: usize>` is a zero-sized type — it has no
fields and occupies no memory in the compiled program. Its parameter `N` is
a *const generic*: Rust generics can be parameterized by constant values as
well as by types, and here the physical pin number is part of the type
itself, so `PinHandle<25>` and `PinHandle<24>` are two distinct types and
passing one where the other is required is a type error. The handle's only
constructor, `PinHandle::new`, is declared `unsafe`, which means safe code
cannot call it at all; the only place it is invoked is inside the code
generated by the `api::define_board!` macro, which a firmware crate invokes
once with the pin map of a real board — so a handle can only come from a
board's `take()`. The macro generates a `Pins` struct containing exactly one
`PinHandle` field per pin that physically exists on that board, wrapped in a
board struct whose `take()` hands the whole struct out at most once per
boot. The same invocation can list the board's peripherals in a `devices`
section; each entry becomes a board field of type `DeviceHandle<T>` — the
peripheral-level analog of a pin handle, a zero-sized claim on the one
peripheral driven by the driver type `T`, which that driver's constructor
consumes. `take()` returns `Option<Self>` — `Option` is Rust's built-in type for
a value that may be absent, with two variants: `Some(value)` and `None`.
The once-only behavior rests on an `AtomicBool`, a boolean flag whose
operations each execute as one indivisible step even with multiple CPU cores
running; `take()` performs a *compare-exchange* on it, which checks that the
flag is `false` and sets it `true` in that single step. A plain
`if !taken { taken = true; ... }` is two separate steps, and two cores could
both pass the check before either sets the flag; with compare-exchange,
even simultaneous calls yield exactly one `Some` — every other and every
later caller gets `None`.

Holding a `PinHandle<25>`, then, proves two things by two different
mechanisms. That pin 25 exists on this board is checked at compile time: the
generated `Pins` struct has a field only for each pin that exists, so there
is no way to obtain a `PinHandle<40>`. That no other code path holds one is
a runtime-plus-contract property: `take()` succeeds once, and because
`PinHandle::new` is `unsafe`, safe code cannot construct a second handle —
an `unsafe` block could, and not doing so is the stated contract on every
`unsafe` block in this tree. `firmware/pico2/src/common/board.rs`
instantiates the macro as `Rp2350`/`Rp2350Pins`: `gpio0`–`gpio22` for the
free header pins, then `smps_ps` (23), `vbus_sense` (24), and `led` (25) —
named for the on-board circuits the Pico 2 commits them to; see the
[Pico 2 datasheet](docs/pico-2-datasheet.pdf) for the power-supply (SMPS),
USB-voltage (VBUS), and system-voltage (VSYS) circuits behind those names —
then `gpio26`–`gpio28`, which can also feed the chip's ADC
(analog-to-digital converter: it measures the voltage on a pin and returns
it as a number), and `vsys_adc` (29).

**Configuration consumes the proof.**
`api::gpio::Gpio` is the factory trait a GPIO driver implements. One Rust
rule makes this invariant work, and it is where Rust departs from C: passing
a value of a non-copyable type to a function does not copy it, it *moves*
it — ownership transfers into the function, the caller's variable is
invalidated, and any later use of that variable is a compile error.
`PinHandle` is non-copyable, and the factory's two methods,
`input_from_handle(handle, pull)` and `output_from_handle(handle)`, take it
**by value**: configuring a pin consumes the handle, so using the same
handle to configure the pin twice is a compile error, not a runtime check.
What you get back is named by the trait's *associated types* — placeholder
types declared inside a trait that each implementing type must fill in with
concrete types of its own. Here the placeholders themselves take the
pin-number parameter (*generic* associated types):
`type Input<const N: usize>: GpioPinIn<N>` and
`type Output<const N: usize>: GpioPinOut<N>`, so an implementation supplies
one concrete pin type per pin number. `GpioPinIn`
and `GpioPinOut` are marker traits — traits with no methods of their own,
existing to name a capability — over `api::common::Read<bool>` and
`api::common::Write<bool>`, so a configured pin is used through the same
portable `read`/`write` traits any other peripheral would use.
`api::common::ErrorType` gives each peripheral a single associated error
type that all of its operations agree on.

**An operation that cannot fail returns an error type with no values.**
The RP2350 implementation, `pico2::gpio::gpio::Rp2350Gpio`, is zero-sized;
its `new(board.gpio)` performs the reset bring-up of the `IO_BANK0` and
`PADS_BANK0` blocks and consumes the board's `DeviceHandle<Rp2350Gpio>` —
so a second construction is a compile error (use of a moved value), the
once-per-boot property needs no runtime check of its own, and `new` returns
`Self` rather than an `Option`. The pin types it
hands out, `Rp2350GpioIn<N>` and `Rp2350GpioOut<N>`, declare
`Error = core::convert::Infallible` — an enum with no variants. Unlike a C
enum, whose values are just integers whether or not any enumerator names
them, a Rust enum value must be one of the enum's declared variants, so a
zero-variant enum has no values at all: the `Err` case of a
`Result<T, Infallible>` can never be constructed, and the compiler removes
that branch. Every failure mode was already handled before the pin existed;
the type signature says so.

The blinky in `templates/pico2/src/main.rs` is the whole pattern. Its `main`, minus the
busy-wait `delay()`, is:

```rust
fn main() -> ! {
    let board = Rp2350::take().unwrap();
    let mut gpio = Rp2350Gpio::new(board.gpio);
    let mut led = gpio.output_from_handle(board.pins.led).unwrap();
    loop {
        led.write(true);
        delay();
        led.write(false);
        delay();
    }
}
```

Two of the setup calls can report failure — `take()` returns the `Option`
described above, and `output_from_handle` returns a `Result` — and the blinky
`.unwrap()`s each one, meaning: halt if the call failed. That is the right
choice here: the `Option` is `None` only on a second call to a
once-per-boot constructor, which is a program bug, not a condition to
recover from. `Rp2350Gpio::new` has no failure case at all — its
once-per-boot property is enforced by consuming `board.gpio`, at compile
time. The same
listing, annotated line by line, is the crate-level doc of
`firmware/pico2/src/lib.rs` ("Writing an application"). Run
`cargo doc -p api --open`, or
`cargo doc -p pico2 --target thumbv8m.main-none-eabihf --open`, to browse
the API documentation; when wiring your own
*input* pin, read `api::gpio::Pull` there (source: `api/src/gpio/mod.rs`),
which explains how to choose the pull configuration for a button or other
input circuit.

## Prerequisites

1. **Rust**, installed via [rustup](https://rustup.rs). Verified with
   `rustc 1.98.0`.
2. **The cross-compilation target.** The RP2350's Arm cores are Cortex-M33
   processors; Rust names that platform with the *target triple*
   `thumbv8m.main-none-eabihf`. Install the precompiled core library for it:

   ```sh
   rustup target add thumbv8m.main-none-eabihf
   ```

3. **picotool** (Raspberry Pi's tool for inspecting and converting RP2350
   images), only needed for flashing. On macOS: `brew install picotool`.
   Verified with picotool v2.3.0.
4. **cargo-generate** 0.23.4 or later, only needed to create an
   application from `templates/`: `cargo install cargo-generate`. Verified
   with cargo-generate 0.23.14.

## Building

The workspace itself is architecture-neutral: `.cargo/config.toml` sets no
target, only an alias. From the repository root:

```sh
cargo build                 # api, for the host (the default member)
cargo pico2                 # pico2, for thumbv8m.main-none-eabihf
```

`cargo pico2` is `cargo build -p pico2 --target thumbv8m.main-none-eabihf`.
Both crates are libraries, so neither command links a firmware image.

An application is built from its own crate. Generate one from the Pico 2
template, then build it there:

```sh
cargo generate --git https://github.com/robinonsay/junoos templates/pico2
cd <project-name>
cargo build --release
```

The generated `.cargo/config.toml` sets the target triple and the linker
arguments (`-Tlink.ld`, a memory report, and a `firmware.map` link map),
and `pico2`'s `build.rs` puts `link.ld` on the linker search path. The
linker prints the memory report as part of the build. For the template's
blinky, as of this commit, it reads:

```
Memory region         Used Size  Region Size  %age Used
            FLASH:        2028 B         4 MB      0.05%
              RAM:        8200 B       520 KB      1.54%
```

(These numbers change whenever the code does; the report printed by your own
build is the authoritative one.) The build also emits one `linker_messages`
warning attributed to the application crate — that is expected; it is how
Cargo surfaces the memory report above.

## Flashing

Hold the **BOOTSEL** button on the Pico 2 while plugging in its USB cable,
then, from the application's directory:

```sh
cargo run --release
```

The template sets `picotool load -u -x -t elf` as the runner, so this
builds the image, writes it to flash (`-u` skips sectors that already
match), and starts it (`-x`). The onboard LED blinks.

To flash without picotool, convert the image to **UF2**, the format the
Pico 2's bootrom accepts by drag-and-drop. The build output is an **ELF**
file (the standard executable format the linker produces), with no
extension; picotool identifies file types by extension, so give it a `.elf`
name before converting:

```sh
cp target/thumbv8m.main-none-eabihf/release/<project-name> app.elf
picotool uf2 convert app.elf app.uf2 --family rp2350-arm-s
```

With the board in BOOTSEL mode it enumerates as a USB mass-storage drive;
copy `app.uf2` onto that drive and the board reboots itself into it.

## Host-side tests

The `api` crate is host-compilable, and its unit tests run on the host:

```sh
cargo test
```

`api` is the workspace's default member, so this runs its unit tests and
doctests. If a `.cargo/config.toml` in an enclosing directory sets a
build target (as one does when this repository is a submodule of a
firmware project), name the host's own triple instead, for example
`cargo test -p api --target aarch64-apple-darwin`.

## Learning path

The main learning path is the bare-metal tutorial:

- **[Bare-Metal Rust on the Raspberry Pi Pico 2](docs/tutorials/rp2350_baremetal/index.md)**
  — nine chapters, from an empty directory to the blinking LED: toolchain
  setup, linker scripts, the RP2350 memory map, boot metadata and vector
  tables, the reset handler, register-level GPIO. Where a tutorial listing
  disagrees with the code in the tree (the workspace has been restructured
  since parts of it were written), the tree is authoritative.

The primary hardware references live in `docs/`:
`docs/rp2350-datasheet.pdf` (the microcontroller),
`docs/pico-2-datasheet.pdf` (the board), the Armv8-M and Cortex-M33
manuals, and the Pico C SDK and getting-started guides, with text
extractions of each under `docs/extracted/`. The drivers cite them by
section, table and page. `docs/` also holds the Pico 2 pinout, and the
datasheets and specifications for the sensors, radio, GPS and SD card that
applications built on JunoOS drive.

## What is implemented

**Implemented today:** the two-crate workspace and the templates described
above — the boot path; the GPIO, clock, timer, SysTick, watchdog, USB
serial, PWM, DMA, ADC, UART (polled and DMA), I2C, SPI and second-core
drivers; the portable traits and the board-definition macro; the app,
iLoad, scheduler, software-bus and time framework; and the blinky and
app-module templates. That is all of the code.

## License

The code is under the [MIT License](LICENSE). The third-party documents in
`docs/` (datasheets, reference manuals and their text extractions) are not:
they remain their publishers' and are included for reference only.
