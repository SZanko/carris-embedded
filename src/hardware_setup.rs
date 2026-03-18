use bt_hci::controller::ExternalController;
use embassy_net::{DhcpConfig, Runner, Stack, StackResources};
use esp_hal::Async;
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::peripherals::{GPIO5, GPIO6, I2C0, Peripherals, WIFI};
use esp_hal::rng::Rng;
use esp_hal::time::Rate;
use esp_radio::ble::controller::BleConnector;
use esp_radio::wifi::{WifiController, WifiDevice};
use ssd1306::mode::{BufferedGraphicsMode, DisplayConfig};
use ssd1306::prelude::{Brightness, DisplayRotation, DisplaySize128x64, I2CInterface};
use ssd1306::{I2CDisplayInterface, Ssd1306};
use trouble_host::HostResources;
use trouble_host::prelude::DefaultPacketPool;

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 1;

pub async fn setup_bluetooth(
    bt: esp_hal::peripherals::BT<'_>,
    radio_init: &esp_radio::Controller<'_>,
) {
    let transport = BleConnector::new(radio_init, bt, Default::default()).unwrap();
    let ble_controller = ExternalController::<_, 1>::new(transport);
    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let _stack = trouble_host::new(ble_controller, &mut resources);
}

macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

pub struct WifiSetup {
    pub controller: WifiController<'static>,
    pub stack: Stack<'static>,
    pub runner: Runner<'static, WifiDevice<'static>>,
    pub tls_seed: u64,
}

pub async fn setup_wifi(
    wifi: WIFI<'static>,
    radio_init: &'static esp_radio::Controller<'_>,
) -> WifiSetup {
    let (mut wifi_controller, interfaces) =
        esp_radio::wifi::new(&radio_init, wifi, Default::default())
            .expect("Failed to initialize Wi-Fi controller");

    let rng = Rng::new();
    let net_seed = rng.random() as u64 | ((rng.random() as u64) << 32);
    let tls_seed = rng.random() as u64 | ((rng.random() as u64) << 32);

    let dhcp_config = DhcpConfig::default();
    let config = embassy_net::Config::dhcpv4(dhcp_config);

    let (stack, runner) = embassy_net::new(
        interfaces.sta,
        config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        net_seed,
    );

    return WifiSetup {
        controller: wifi_controller,
        stack,
        runner,
        tls_seed,
    };
}

pub async fn setup_integrated_display_esp32c3<'d>(
    i2c0: I2C0<'d>,
    gpio5: GPIO5<'d>,
    gpio6: GPIO6<'d>,
) -> Ssd1306<I2CInterface<I2c<'d, Async>>, DisplaySize128x64, BufferedGraphicsMode<DisplaySize128x64>>
{
    let i2c_config = I2cConfig::default().with_frequency(Rate::from_khz(400));

    let i2c = I2c::new(i2c0, i2c_config)
        .unwrap()
        .with_sda(gpio5)
        .with_scl(gpio6)
        .into_async();

    let interface = I2CDisplayInterface::new(i2c);
    let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
        .into_buffered_graphics_mode();
    display.init().unwrap();

    display.set_brightness(Brightness::BRIGHTEST).unwrap();
    return display;
}
