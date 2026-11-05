// Copyright 2016-2018 Mateusz Sieczko and other GilRs Developers
//
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.
//
// PATCHED (tdug/n64_controller_plugin): upstream gated all force feedback on
// `Windows.Gaming.Input.Gamepad`, which only exists for devices Windows classifies as
// XInput-class. A Bluetooth pad that only shows up as a generic `RawGameController` (SDL
// mappings) has real `ForceFeedbackMotors()` but no `Gamepad` handle, so upstream can never
// drive it. This adds a fallback that drives those motors directly via a `ConstantForceEffect`.
// See https://gitlab.com/gilrs-project/gilrs/-/issues/51 and tdug/n64_controller_plugin#3.
use std::time::Duration;
use windows::Foundation::TimeSpan;
use windows::Gaming::Input::ForceFeedback::ConstantForceEffect;
use windows::Gaming::Input::Gamepad as WgiGamepad;
use windows::Gaming::Input::GamepadVibration;
use windows::Gaming::Input::RawGameController;
use windows_numerics::Vector3;

#[derive(Debug)]
pub struct Device {
    id: u32,
    wgi_gamepad: Option<WgiGamepad>,
    raw_game_controller: Option<RawGameController>,
    raw_effect: Option<ConstantForceEffect>,
}

impl Device {
    pub(crate) fn new(
        id: u32,
        wgi_gamepad: Option<WgiGamepad>,
        raw_game_controller: Option<RawGameController>,
    ) -> Self {
        Device {
            id,
            wgi_gamepad,
            raw_game_controller,
            raw_effect: None,
        }
    }

    pub fn set_ff_state(&mut self, strong: u16, weak: u16, _min_duration: Duration) {
        if let Some(wgi_gamepad) = &self.wgi_gamepad {
            if let Err(err) = wgi_gamepad.SetVibration(GamepadVibration {
                LeftMotor: (strong as f64) / (u16::MAX as f64),
                RightMotor: (weak as f64) / (u16::MAX as f64),
                LeftTrigger: 0.0,
                RightTrigger: 0.0,
            }) {
                error!(
                    "Failed to change FF state – unknown error. ID = {}, error = {:?}.",
                    self.id, err
                );
            }
            return;
        }
        self.set_raw_ff_state(strong, weak);
    }

    /// Fallback for pads with no WGI `Gamepad` handle: drive the first motor the generic
    /// `RawGameController` interface reports directly.
    fn set_raw_ff_state(&mut self, strong: u16, weak: u16) {
        let Some(raw) = &self.raw_game_controller else {
            return;
        };
        let magnitude = (strong.max(weak) as f32) / (u16::MAX as f32);

        let effect = match &self.raw_effect {
            Some(effect) => effect,
            None => {
                let motor = match raw.ForceFeedbackMotors().and_then(|motors| motors.GetAt(0)) {
                    Ok(motor) => motor,
                    Err(err) => {
                        error!(
                            "Failed to get force feedback motor. ID = {}, error = {:?}.",
                            self.id, err
                        );
                        return;
                    }
                };
                let effect = match ConstantForceEffect::new() {
                    Ok(effect) => effect,
                    Err(err) => {
                        error!(
                            "Failed to create force feedback effect. ID = {}, error = {:?}.",
                            self.id, err
                        );
                        return;
                    }
                };
                // Loading the effect is async; `.get()` blocks this call until it completes,
                // same as every other set_ff_state backend in gilrs-core.
                if let Err(err) = motor
                    .LoadEffectAsync(&effect)
                    .and_then(|op| op.get().map(|_| ()))
                {
                    error!(
                        "Failed to load force feedback effect. ID = {}, error = {:?}.",
                        self.id, err
                    );
                    return;
                }
                self.raw_effect = Some(effect);
                self.raw_effect.as_ref().unwrap()
            }
        };

        // 100-day constant pull in the X axis; magnitude is all we actually control, and
        // set_ff_state is called continuously while rumble is active, same as the XInput path.
        let vector = Vector3 {
            X: magnitude,
            Y: 0.0,
            Z: 0.0,
        };
        let hundred_days = TimeSpan {
            Duration: 100 * 24 * 60 * 60 * 10_000_000,
        };
        if let Err(err) = effect.SetParameters(vector, hundred_days) {
            error!(
                "Failed to set force feedback parameters. ID = {}, error = {:?}.",
                self.id, err
            );
            return;
        }
        let result = if magnitude > 0.0 {
            effect.Start()
        } else {
            effect.Stop()
        };
        if let Err(err) = result {
            error!(
                "Failed to change FF state – unknown error. ID = {}, error = {:?}.",
                self.id, err
            );
        }
    }
}
