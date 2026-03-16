#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use alloc::boxed::Box;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;

use bt_hci::controller::ExternalController;
use esp_radio::ble::controller::BleConnector;
use trouble_host::prelude::*;

use defmt::info;
use esp_println as _;

use embassy_executor::Spawner;
use embassy_net::{
    DhcpConfig, Runner, Stack, StackResources,
    dns::DnsSocket,
    tcp::client::{TcpClient, TcpClientState},
};
use embassy_time::{Duration, Timer};

use carris_embedded::hardware_setup::{
    setup_bluetooth, setup_integrated_display_esp32c3, setup_wifi,
};
use embedded_graphics::{
    mono_font::{MonoTextStyle, MonoTextStyleBuilder, ascii::FONT_6X10},
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text},
};
use esp_backtrace as _;
use esp_hal::Async;
use esp_hal::analog::adc::{Adc, AdcConfig, Attenuation};
use esp_hal::peripherals::Peripherals;
use esp_radio::wifi::{
    ClientConfig, ModeConfig, ScanConfig, WifiController, WifiDevice, WifiEvent, WifiStaState,
};
use reqwless::client::{HttpClient, TlsConfig};
use ssd1306::{I2CDisplayInterface, Ssd1306, mode::BufferedGraphicsMode, prelude::*};

extern crate alloc;

// Do not convert to unwrap
const SSID: &str = match option_env!("SSID") {
    Some(v) => v,
    None => "OpenFCT",
};
const PASSWORD: &str = match option_env!("PASSWORD") {
    Some(v) => v,
    None => "",
};

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

//#[embassy_executor::task]
//async fn ldr_task(
//    mut adc: Adc<'static, esp_hal::peripherals::ADC1>,
//    mut adc_pin: esp_hal::analog::adc::AdcPin<esp_hal::peripherals::GPIO0, esp_hal::peripherals::ADC1>,
//) {
//    loop {
//        let value: u16 = adc.read(&mut adc_pin).unwrap();
//        info!("LDR value: {}", value);
//
//        if value < 1000 {
//            info!("Dark");
//        } else {
//            info!("Bright");
//        }
//
//        Timer::after(Duration::from_millis(500)).await;
//    }
//}

fn write_to_integrated_display_esp32c3(
    display: &mut Ssd1306<
        I2CInterface<I2c<Async>>,
        DisplaySize128x64,
        BufferedGraphicsMode<DisplaySize128x64>,
    >,
    text_style: MonoTextStyle<'_, BinaryColor>,
    line1: &str,
    line2: &str,
    line3: &str,
) {
    let width: i32 = 72;
    let _height: i32 = 40;
    let x_offset: i32 = 28;
    let y_offset: i32 = 16;

    // FONT_6X10 is 6 px wide, 10 px high per char
    let w1 = (line1.len() as i32) * 6;
    let w2 = (line2.len() as i32) * 6;

    let x1 = x_offset + (width - w1) / 2;
    let y1 = y_offset + 18;

    let x2 = x_offset + (width - w2) / 2;
    let y2 = y_offset + 32;

    let x3 = x_offset + (width - w2) / 2;
    let y3 = y_offset + 46;

    Text::with_baseline(line1, Point::new(x1, y1), text_style, Baseline::Bottom)
        .draw(display)
        .unwrap();

    Text::with_baseline(line2, Point::new(x2, y2), text_style, Baseline::Bottom)
        .draw(display)
        .unwrap();

    Text::with_baseline(line3, Point::new(x3, y3), text_style, Baseline::Bottom)
        .draw(display)
        .unwrap();

    info!(
        "Write to integrated display \n Line 1: {} Line 2: {} Line 3: {}",
        line1, line2, line3
    );

    display.flush().unwrap();
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}

async fn access_website(stack: Stack<'_>, tls_seed: u64) {
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];
    let dns = DnsSocket::new(stack);
    let tcp_state = TcpClientState::<1, 4096, 4096>::new();
    let tcp = TcpClient::new(stack, &tcp_state);

    let tls = TlsConfig::new(
        tls_seed,
        &mut rx_buffer,
        &mut tx_buffer,
        reqwless::client::TlsVerify::None,
    );

    let mut client = HttpClient::new_with_tls(&tcp, &dns, tls);
    let mut buffer = [0u8; 4096];
    let mut http_req = client
        .request(
            reqwless::request::Method::GET,
            "https://jsonplaceholder.typicode.com/posts/1",
        )
        .await
        .unwrap();
    let response = http_req.send(&mut buffer).await.unwrap();

    info!("Got response");
    let res = response.body().read_to_end().await.unwrap();

    let content = core::str::from_utf8(res).unwrap();
    info!("{}", content);
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    info!("start connection task");
    info!("Device capabilities: {:?}", controller.capabilities());
    loop {
        match esp_radio::wifi::sta_state() {
            WifiStaState::Connected => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = ModeConfig::Client(
                ClientConfig::default()
                    .with_ssid(SSID.into())
                    .with_password(PASSWORD.into()),
            );
            controller.set_config(&client_config).unwrap();
            info!("Starting wifi");
            controller.start_async().await.unwrap();
            info!("Wifi started!");

            info!("Scan");
            let scan_config = ScanConfig::default().with_max(10);
            let result = controller
                .scan_with_config_async(scan_config)
                .await
                .unwrap();
            for ap in result {
                info!("{:?}", ap);
            }
        }
        info!("About to connect...");

        match controller.connect_async().await {
            Ok(_) => info!("Wifi connected!"),
            Err(e) => {
                info!("Failed to connect to wifi: {:?}", e);
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

async fn wait_for_connection(stack: Stack<'_>) {
    info!("Waiting for link to be up");
    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    info!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            info!("Got IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // generator version: 1.2.0

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 66320);
    // COEX needs more RAM - so we've added some more
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    info!("Embassy initialized!");

    let radio_init = mk_static!(
        esp_radio::Controller<'static>,
        esp_radio::init().expect("Failed to initialize Wi-Fi/BLE controller")
    );
    let wifisetup = setup_wifi(peripherals.WIFI, radio_init).await;
    // find more examples https://github.com/embassy-rs/trouble/tree/main/examples/esp32
    setup_bluetooth(peripherals.BT, radio_init).await;

    spawner.spawn(connection(wifisetup.controller)).ok();
    spawner.spawn(net_task(wifisetup.runner)).ok();
    wait_for_connection(wifisetup.stack).await;

    let mut adc_config = AdcConfig::new();
    let mut ldr_pin = adc_config.enable_pin(peripherals.GPIO2, Attenuation::_11dB);
    let mut adc = Adc::new(peripherals.ADC2, adc_config);

    // TODO: Spawn some tasks
    //spawner.spawn(ldr_task(adc, adc_pin)).unwrap();

    let mut led = Output::new(peripherals.GPIO10, Level::High, OutputConfig::default());

    //let _ = spawner;

    let mut integrated_display =
        setup_integrated_display_esp32c3(peripherals.I2C0, peripherals.GPIO5, peripherals.GPIO6)
            .await;

    let text_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

    loop {
        access_website(wifisetup.stack, wifisetup.tls_seed).await;

        integrated_display.clear(BinaryColor::Off).unwrap();
        led.toggle();

        // Optional frame, same as drawFrame(...)
        // Rectangle::new(
        //     Point::new(x_offset, y_offset),
        //     Size::new(width as u32, height as u32),
        // )
        // .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        // .draw(&mut display)
        // .unwrap();

        write_to_integrated_display_esp32c3(
            &mut integrated_display,
            text_style,
            "Connected to",
            "Accesspoint",
            SSID,
        );

        //let pin_value: u16 = nb::block!(adc.read_oneshot(&mut ldr_pin)).unwrap();
        //info!("LDR reads: {}", pin_value);

        //if pin_value > 3500 {
        //    led.set_high();
        //} else {
        //    led.set_low();
        //}

        Timer::after(Duration::from_secs(10)).await;
    }

    // for inspiration have a look at the examples at https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0/examples
}
