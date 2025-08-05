#![no_std]
#![no_main]

extern crate alloc;

use core::num::NonZeroU8;
use core::num::NonZeroU64;

use alloc::boxed::Box;

use embedded_alloc::LlffHeap as Heap;
use embedded_hal::digital::PinState;
use embedded_hal::digital::{InputPin, OutputPin};
use panic_halt as _;
use rp_pico::entry;
use rp_pico::hal;
use rp_pico::hal::pac;
use rp_pico::hal::reset;
use usb_device::{class_prelude::*, prelude::*};
use usbd_serial::SerialPort;
use usbd_serial::embedded_io::ReadReady;
use usbd_serial::embedded_io::WriteReady;

#[global_allocator]
static HEAP: Heap = Heap::empty();

// trust an input to be done bouncing after it has remained settled for 8ms
const BOUNCE_TIME: u64 = 8000;

struct ReportedControl {
    read_control: Box<dyn FnMut() -> Option<NonZeroU8>>,
    // if report_as is "sb":
    // any state -> b'?': standby_led will begin flashing until we are given a
    //                    positive or negative test result from the host
    // b'?' -> any state: if standby_led was flashing, it will stop
    report_as: &'static str,
    previous_value: Option<NonZeroU8>,
    bounced_by: Option<NonZeroU64>,
}

macro_rules! control {
    ($name:literal, $pin:expr) => {{
        let mut pin = $pin.into_pull_up_input();
        ReportedControl {
            read_control: Box::new(move || {
                if pin.is_low().ok()? {
                    NonZeroU8::new(b'1')
                } else {
                    NonZeroU8::new(b'0')
                }
            }),
            report_as: $name,
            previous_value: None,
            bounced_by: None,
        }
    }};
    ($name:literal, $(=> $defwat:literal,)? $($pin:expr => $wat:literal),+ $(,)?) => {{
        let mut pins = [$({
            let mut pin = $pin.into_pull_up_input();
            Box::new(move || {
                if pin.is_low().ok()? {
                    NonZeroU8::new($wat)
                } else {
                    None
                }
            }) as Box::<dyn FnMut() -> Option<NonZeroU8>>
        }),+];
        ReportedControl {
            read_control: Box::new(move || {
                #[allow(unused)]
                {
                    for pin in pins.iter_mut() {
                        if let Some(result) = pin() {
                            return Some(result)
                        }
                    }
                    $(return NonZeroU8::new($defwat);)?
                    None
                }
            }),
            report_as: $name,
            previous_value: None,
            bounced_by: None,
        }
    }};
}

fn patiently_write(
    mut w: impl FnMut(&[u8]) -> Option<usize>,
    mut bytes: &[u8],
) {
    while !bytes.is_empty() {
        let Some(wrote) = w(bytes) else {
            return;
        };
        bytes = &bytes[wrote..];
    }
}

#[entry]
fn main() -> ! {
    #[allow(static_mut_refs)]
    {
        use core::mem::MaybeUninit;
        const HEAP_SIZE: usize = 131072;
        static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] =
            [MaybeUninit::uninit(); HEAP_SIZE];
        unsafe { HEAP.init(HEAP_MEM.as_ptr() as usize, HEAP_SIZE) }
    }
    let mut pac = pac::Peripherals::take().unwrap();
    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);
    let clocks = hal::clocks::init_clocks_and_plls(
        rp_pico::XOSC_CRYSTAL_FREQ,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();
    let timer = hal::Timer::new(pac.TIMER, &mut pac.RESETS, &clocks);
    let usb_bus = UsbBusAllocator::new(hal::usb::UsbBus::new(
        pac.USBCTRL_REGS,
        pac.USBCTRL_DPRAM,
        clocks.usb_clock,
        true,
        &mut pac.RESETS,
    ));
    let mut serial = SerialPort::new(&usb_bus);
    let mut usb_dev =
        UsbDeviceBuilder::new(&usb_bus, UsbVidPid(0x16c0, 0x27dd))
            .strings(&[StringDescriptors::default()
                .manufacturer("Tejat !INC")
                .product("External Skyhawk")
                .serial_number("N172SP")])
            .unwrap()
            .device_class(2)
            .build();
    let sio = hal::Sio::new(pac.SIO);
    let pins = rp_pico::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );
    let mut controls = [
        control!("bat", pins.gpio0), // battery
        control!("alt", pins.gpio1), // alternator
        control!("av1", pins.gpio2), // avionics bus 1
        control!("av2", pins.gpio3), // avionics bus 2
        control!("ph", pins.gpio4),  // pitot heaters
        control!("fp", pins.gpio5),  // electric fuel pump
        control!("lb", pins.gpio6),  // beacon (ground recognition) light
        control!("ll", pins.gpio7),  // landing lights
        control!("lt", pins.gpio8),  // taxi light
        control!("ln", pins.gpio9),  // nav lights
        control!("ls", pins.gpio10), // strobe (anti-collision) lights
        control!(
            "mag", // magnetos (ignition switch)
            pins.gpio11 => b'0', // off
            pins.gpio12 => b'1', // L
            pins.gpio13 => b'2', // R
            pins.gpio14 => b'3', // both
            pins.gpio15 => b'4', // ignition
        ),
        control!(
            "fl", // flap configuration
            => b'0', // neutral
            pins.gpio16 => b'-', // retract
            pins.gpio17 => b'+', // extend
        ),
        control!(
            "sb", // standby instrument power + annunciator test switch
            => b'0', // neutral
            pins.gpio18 => b'1', // activate
            pins.gpio19 => b'?', // test
        ),
        control!(
            "pb", // parking brake
            pins.gpio20 => b'-', // disengage
            pins.gpio21 => b'1', // engage
        ),
    ];
    let mut on_board_led = pins.led.into_push_pull_output();
    let mut standby_state = Some(false);
    let mut standby_test_led =
        pins.gpio22.into_push_pull_output_in_state(PinState::Low);
    let mut prev_scan_index = usize::MAX;
    loop {
        let _ = usb_dev.poll(&mut [&mut serial]);
        if matches!(serial.read_ready(), Ok(true)) {
            let mut buf = [0u8];
            if let Ok(1) = serial.read(&mut buf) {
                match buf[0] {
                    b'!' => {
                        standby_state = Some(true);
                    }
                    b'.' => {
                        standby_state = Some(false);
                    }
                    b'?' => {
                        patiently_write(
                            |b| {
                                usb_dev.poll(&mut [&mut serial]);
                                serial.write(b).ok()
                            },
                            b"We are a Cessna 172 SP?\n",
                        );
                        for control in controls.iter_mut() {
                            control.previous_value = None;
                        }
                        standby_state = Some(false);
                    }
                    b'r' => reset(),
                    _ => (),
                }
            }
        }
        if !matches!(serial.write_ready(), Ok(true)) {
            continue;
        }
        let now = timer.get_counter().ticks();
        let scan_index = (now / 10_000_000 % controls.len() as u64) as usize;
        let mut any_lit = false;
        for (i, control) in controls.iter_mut().enumerate() {
            any_lit = any_lit
                || control
                    .previous_value
                    .map(|x| x.get() != b'0')
                    .unwrap_or(false);
            let Some(nu) = (control.read_control)() else {
                continue;
            };
            if Some(nu) != control.previous_value {
                match control.bounced_by {
                    None => {
                        control.bounced_by =
                            NonZeroU64::new(now + BOUNCE_TIME);
                        continue;
                    }
                    Some(bounced_by) => {
                        if bounced_by.get() > now {
                            continue;
                        }
                        control.bounced_by = None;
                    }
                }
            } else {
                control.bounced_by = None;
            }
            if control.report_as == "sb" && Some(nu) != control.previous_value
            {
                if nu.get() == b'?' {
                    standby_state = None;
                } else if standby_state.is_none() {
                    standby_state = Some(false);
                }
            }
            if Some(nu) != control.previous_value
                || (scan_index == i && scan_index != prev_scan_index)
            {
                control.previous_value = Some(nu);
                patiently_write(
                    |b| {
                        usb_dev.poll(&mut [&mut serial]);
                        serial.write(b).ok()
                    },
                    control.report_as.as_bytes(),
                );
                patiently_write(
                    |b| {
                        usb_dev.poll(&mut [&mut serial]);
                        serial.write(b).ok()
                    },
                    &[b'=', nu.get(), b'\n'],
                );
            }
        }
        prev_scan_index = scan_index;
        let standby_on = standby_state.unwrap_or(now / 100000 % 2 != 0);
        let _ = standby_test_led.set_state(standby_on.into());
        on_board_led
            .set_state(
                (any_lit || (standby_on && standby_state.is_none())).into(),
            )
            .unwrap();
    }
}
