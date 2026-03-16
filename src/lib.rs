#![no_std]

use embedded_graphics::Drawable;
use embedded_graphics::geometry::Point;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::text::{Baseline, Text, TextStyle};
use esp_hal::Async;
use esp_hal::i2c::master::I2c;
use ssd1306::mode::BufferedGraphicsMode;
use ssd1306::prelude::I2CInterface;
use ssd1306::size::DisplaySize128x64;
use ssd1306::Ssd1306;
