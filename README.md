This is firmware for a Raspberry Pi Pico, allowing it to act as the interface to a simulated [Cessna 172SP](https://en.wikipedia.org/wiki/Cessna_172) front panel.

# How to install?

- Install [the prerequisites for Raspberry Pi Pico Rust development](https://github.com/rp-rs/rp-hal#getting-started), including elf2uf2-rs.
- Connect a Raspberry Pi Pico to your computer via USB while holding the BOOTSEL button down.
- `cargo run --release` in this directory.

# How to talk to it?

The Raspberry Pi Pico will now present a serial port to the host PC. You run an interface program, or a flight simulator plugin, that will access this serial port.

The below information will be relevant if you're writing your own interface program.

## Writing

Each byte you write to the serial port is a command:

- `?`: Identify yourself! (Device will respond with a line `"We are a Cessna 172 SP?"` and then report the current state of all switches.)
- `!`: Turn the standby power test LED on
- `.`: Turn the standby power test LED off
- `r`: Reset the RP2040—if the BOOTSEL button is held down, this will put it back into bootstrap mode.

You can set the state of the standby power test LED at any time, but if you receive a `sb=?` report from the device, you should immediately tell it what the current state of the standby power test LED should be. If you don't, the device will rapidly blink that LED (as well as the one on the board itself) to inform the user that the host software is not handling the test correctly (possibly because it's not running).

## Reading

Everything sent back to you over the serial port will be an entire line, delimited by a linefeed (`'\n'`). See Wiring for the list of reports that will normally be sent. (When no method of sending `xxx=0` is specified, it will be sent when the switch is open.)

# Wiring

It expects the switches to be connected between GND and the following GPIO pins:

- 0: `bat=1`: Battery
- 1: `alt=1`: Alternator
- 2: `av1=1`: Avionics bus 1
- 3: `av2=1`: Avionics bus 2
- 4: `ph=1`: Pitot heaters
- 5: `fp=1`: Electric fuel pump
- 6: `lb=1`: Beacon (ground recognition light)
- 7: `ll=1`: Landing lights
- 8: `lt=1`: Taxi lights
- 9: `ln=1`: Nav lights
- 10: `ls=1`: Strobe (anti-collision) lights
- 11: `mag=0`: Magneto OFF
- 12: `mag=1`: Magneto LEFT (optional)
- 13: `mag=2`: Magneto RIGHT (optional)
- 14: `mag=3`: Magneto BOTH
- 15: `mag=4`: Magneto START
- 16: `fl=+`: Flaps RETRACT (optional)
- 17: `fl=-`: Flaps EXTEND (optional)
- 18: `sb=1`: Standby instrument power on (G1000 only, optional)
- 19: `sb=?`: Standby instrument power + anunciator test (G1000 only, optional)
- 20: `pb=-`: Disengage parking brake (optional)
- 21: `pb=1`: Engage parking brake (optional)

It will output a logic HIGH on GPIO 22 if the standby power test LED should be lit. You may connect this to a green LED with an appropriate resistor; please limit the current!

Most switches should be normally-open toggle switches. Exceptions:

- Magneto: should be at least a 3-position switch, which connects at least one of 11, 14, or 15 to GND in any valid position. (LEFT/RIGHT positions are optional.)
- Flaps: should be two momentary switches, or one two-sided momentary switch.
- Standby: should be one toggle (on) and one momentary (test) switch, or one two-sided toggle (whose TEST side may optionally be momentary)
- Parking brake: should be two momentary switches, or one two-sided momentary switch, or—because you can disengage the parking brake by stepping on the toe brake pedals—just one momentary switch.

# License

Public domain.
