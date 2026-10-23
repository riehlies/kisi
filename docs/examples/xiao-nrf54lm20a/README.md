# Worked example: Seeed Studio XIAO nRF54LM20A

`XIAO nRF54LM20A.kisi.json` is a board configuration for kisi. It describes the board's **power tree for the
USB supply case**, and every value carries its source. The kisi-prepare skill wrote it from the datasheets, and
it was then cleaned up by hand. Use it as a pattern when you prepare a board of your own: it shows a PMIC with
several channels, load switches, a 0 Ω link and ferrite beads as pass elements, hierarchical net names in
quotes, and loads marked as typical, maximum or estimated.

## The board
- **Seeed Studio XIAO nRF54LM20A**, hardware V1.0: the KiCad 10 project `XIAO_nRF54LM20A_V1.0_SCH&PCB_260508`.
  Seeed publishes one project for the XIAO nRF54LM20A and the Sense variant, so the board file includes the IMU
  and the microphone.
- The hardware files are by Seeed Studio, licensed CC BY-SA 4.0, and are **not included here**. Get them from
  the Resources section of the
  [XIAO nRF54LM20A wiki page](https://wiki.seeedstudio.com/xiao_nrf54lm20a_getting_started/) ("XIAO nRF54LM20A
  KiCad Project",
  [direct link](https://files.seeedstudio.com/wiki/XIAO_nRF54LM20A/getting_start/RES/XIAO_nRF54LM20A_V1.0_SCH&PCB_260508.zip)).

## The power tree (USB case)
```
USB1 USB-C: TYPEC_5V, 5 V input
└ U6 TPS22916 load switch → VBUS
  └ U2 nPM1300: VSYS follows VBUS (5 V while USB is present)
    ├ U2 BUCK2 3.3 V → VSYS_3V3: U8 nRF54LM20A, U10 flash, LED1, U4 level translator
    │  └ U3 TPS22916 load switch → SAMD11_3V3: U5 SAMD11 debugger, U4
    ├ R14 0 Ω → /03 Power/LDO1_IN
    │  └ U2 LDO1 3.3 V → IMU&MIC_3V3
    │     ├ FB3 → /06 Peripherals/IMU_VDD: U11 LSM6DS3TR-C
    │     └ FB4 → /06 Peripherals/MIC_VDD: U12 MSM261DGT006
    └ U1 TPS628438 3.3 V → 3V3_OUT (header pin only; no load modelled)
```
On battery, VSYS is VBAT (3.0–4.2 V) and the SAMD11 debugger is off. kisi has one tree per board for now
(ROADMAP: power tree per supply case).

## Which values come from where
| Kind | Values |
|---|---|
| Datasheet | U6 and U3 on-resistance (80 and 90 mΩ maximum) and the 2 A rating; nPM1300 BUCK2 200 mA and its 3.3 V setting, LDO1 50 mA with 12.5 µA ground current, VSYS following VBUS (Nordic DevZone answers by Nordic staff); TPS628438 3.3 V setting and 600 mA; U8 12 mA (radio TX at maximum power plus CPU, typical); U4 12 µA + 2.4 µA; U11 0.9 mA (typical); FB3/FB4 120 Ω at 100 MHz and 0.5 Ω DC resistance (maximum) |
| Estimate | U10 6 mA (from the datasheet of a sibling flash, the P25Q64H); U5 4.4 mA (from the SAM D20 table, the same core); U12 0.75 mA (from sibling microphones); LED1 1.2 mA (from its series resistors); the efficiency of BUCK2 and U1, which use kisi's default of 85 % (marked "estimate" in the power tree and in `kisi power`) |
| Assumption | USB1 supplies 5 V; the R14 0 Ω link counts as 0 Ω |

The `note` of each source says whether a value is typical or maximum and at which condition.

## Using it
1. Download and unpack the KiCad project. Copy `XIAO nRF54LM20A.kisi.json` next to `XIAO nRF54LM20A.kicad_pcb`
   (same stem).
2. Command line (`kisi.mjs` from the [latest release](https://github.com/riehlies/kisi/releases/latest)):
   ```sh
   node kisi.mjs config-check "XIAO nRF54LM20A.kicad_pcb"   # ok, 17 entries
   node kisi.mjs power        "XIAO nRF54LM20A.kicad_pcb"   # power tree, IR drop, losses
   node kisi.mjs gaps         "XIAO nRF54LM20A.kicad_pcb"   # what is still open
   ```
3. Viewer: drop the `.kicad_pcb`, the `.kicad_pro` and the `.kisi.json` together into `kisi.html`, then open the
   Power view.

With kisi 0.2.5, `kisi power` gives 100 mW from USB: 83 mW to the loads and 17 mW of losses. BUCK2 loses
14 mW at the estimated 85 % and LDO1 3 mW. The largest DC drop is 0.17 mV, on VSYS_3V3.

## What is still open
`kisi gaps` lists these. They are expected for this example:
- **rails-without-voltage: VBAT.** The battery is not part of the USB-case tree.
- **ic-loads: U7**, the XIAO header. External loads on the 3V3 and VBUS pins are unknown.
- **regulator-efficiency: BUCK2 and U1.** The datasheets give efficiency only as curves.
- **cap-ratings.** The board's capacitors carry `Voltage Rating` and `Dielectric` fields, but kisi does not read
  them yet, so they are not repeated as `PI_CAP` here.
- **stackup-defaults.** The laminate values are KiCad's defaults (FR4, εr 4.5, tanδ 0.02).
- The nPM1300 battery charger is not modelled. While it charges, its current is a load on VSYS.
