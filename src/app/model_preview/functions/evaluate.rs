//! Evaluating an object's functions for a frame.

use super::*;

impl H2ObjectFunction {
    fn is_periodic(&self) -> bool {
        self.function.function_type() == FunctionType::Periodic
    }

    /// `sub_86ECD0`: the value and whether it is active.
    fn evaluate(
        &self,
        owner: &ObjectFunctions,
        inputs: &FunctionValues,
        scripted: &FunctionValues,
        clock: Clock,
        depth: usize,
    ) -> (f32, bool) {
        // A script's value for the import is the function's, unmapped.
        if let Some(value) = scripted.get(&self.import) {
            return (*value, true);
        }
        let (input, mut active) = owner.exported(&self.import, inputs);
        // No name reads full, which is what an object with no such field gets.
        let range = if self.range.is_empty() { 1.0 } else { owner.value(&self.range, inputs, scripted, clock, depth + 1).0 };
        let mapping_passes = self.flags.mapping_does_not_control_active;
        let mut value;
        if self.is_periodic() {
            let time = if self.flags.random_time_offset { clock.time + clock.offset } else { clock.time };
            value = self.function.evaluate(time, range) * input;
        } else if !active && mapping_passes {
            value = input;
        } else {
            let inverted = if self.flags.invert { 1.0 - input } else { input };
            value = self.function.evaluate(inverted, range);
            if !mapping_passes {
                active = value > 0.0;
            }
        }
        if !self.scale_by.is_empty() {
            let (scale, scaled) = owner.value(&self.scale_by, inputs, scripted, clock, depth + 1);
            value *= scale;
            active = active && scaled;
        }
        if active && self.min_value > 0.0 {
            active = value > self.min_value;
        }
        if self.flags.always_active {
            active = true;
        }
        if active && !self.turn_off_with.is_empty() {
            active = match owner.rules {
                FunctionRules::Halo2 => owner.exported(&self.turn_off_with, inputs).1,
                FunctionRules::Gen3 => {
                    let (magnitude, on) = owner.value(&self.turn_off_with, inputs, scripted, clock, depth + 1);
                    on && (!self.flags.turn_off_with_uses_magnitude || magnitude != 0.0)
                }
            };
        }
        match (active, self.flags.always_exports_value) {
            (true, _) => (value.clamp(0.0, 1.0), true),
            (false, true) => (value.clamp(0.0, 1.0), false),
            (false, false) => (0.0, false),
        }
    }
}

impl ObjectFunctions {
    /// What an idle object gives `name`: its type's exporter's value, or,
    /// where those are not known, [`object_input`]'s.
    pub(super) fn idle(&self, name: &str) -> f32 {
        match &self.exports {
            Some(exports) => exports.idle(name).unwrap_or(0.0),
            None => object_input(name),
        }
    }

    /// Whether the object's type exports `name`, so its exporter answers for
    /// it before any function of the same name.
    pub(super) fn type_exports(&self, name: &str) -> bool {
        self.exports.as_ref().is_some_and(|exports| exports.idle(name).is_some())
    }

    /// What `object_type_export_function_values` leaves for `name`: the
    /// panel's value, else the idle one, and whether that is on.
    fn exported(&self, name: &str, inputs: &FunctionValues) -> (f32, bool) {
        let value = inputs.get(name).copied().unwrap_or_else(|| self.idle(name));
        (value, value > 0.0)
    }

    /// Whether a function here makes `name` for what reads it: one of that
    /// name, and none of the type's exporters claiming it first.
    pub(super) fn makes(&self, name: &str) -> bool {
        !self.type_exports(name) && self.exports_named().any(|export| export == name)
    }

    /// Every name a function or switch here exports.
    fn exports_named(&self) -> impl Iterator<Item = &str> {
        let functions = self.functions.iter().map(|function| function.export.as_str());
        functions.chain(self.switches.iter().map(|switch| switch.export.as_str()))
    }

    /// What `name` is made from here: a function's import, scale and
    /// `turn off with`, or a switch's switch function and cases.
    fn sources_of<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        let functions = self.functions.iter().filter(move |function| function.export == name);
        let functions = functions.flat_map(|function| [&function.import, &function.scale_by, &function.turn_off_with]);
        let switches = self.switches.iter().filter(move |switch| switch.export == name);
        let switches = switches.flat_map(|switch| {
            std::iter::once(&switch.switch).chain(switch.cases.iter().map(|(_, function)| function))
        });
        functions.chain(switches).map(String::as_str).filter(|source| !source.is_empty())
    }

    /// `object_get_function_value`: a script's value first
    /// (`object_set_function_variable`, read by Halo 2's `sub_8E7890`), then
    /// nothing named reads full, then what the object's type exports, then
    /// its functions: by [`FunctionRules`], the last or the first of a name.
    /// Halo 4 (the 360's 0x82a92870) asks its switches only when no function
    /// has the name, and every switch of the name runs, so the last stands.
    pub(super) fn value(
        &self,
        name: &str,
        inputs: &FunctionValues,
        scripted: &FunctionValues,
        clock: Clock,
        depth: usize,
    ) -> (f32, bool) {
        if let Some(value) = scripted.get(name) {
            return (*value, true);
        }
        if name.is_empty() || name == "one" {
            return (1.0, true);
        }
        if name == "zero" || depth > DEEPEST_CHAIN {
            return (0.0, false);
        }
        if self.type_exports(name) {
            return self.exported(name, inputs);
        }
        let named = |function: &&H2ObjectFunction| function.export == name;
        let answers = match self.rules {
            FunctionRules::Halo2 => self.functions.iter().rev().find(named),
            FunctionRules::Gen3 => self.functions.iter().find(named),
        };
        if let Some(function) = answers {
            return function.evaluate(self, inputs, scripted, clock, depth);
        }
        match self.switches.iter().rev().find(|switch| switch.export == name) {
            Some(switch) => switch.evaluate(self, inputs, scripted, clock, depth),
            None => self.exported(name, inputs),
        }
    }

    /// The names the panel can set: what a function reads, and what scales or
    /// turns it off, unless another function here makes that.
    pub(super) fn imports(&self) -> Vec<&str> {
        let mut names: Vec<&str> = Vec::new();
        for export in self.exports_named() {
            for name in self.sources_of(export) {
                if !self.makes(name) && !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        names
    }

    /// Everything `read` depends on, down through the functions making it.
    pub(super) fn reaching(&self, read: &[String]) -> Vec<String> {
        let mut reached = read.to_vec();
        let mut next = 0;
        while let Some(name) = reached.get(next).cloned() {
            next += 1;
            for source in self.sources_of(&name) {
                if !reached.iter().any(|reached| reached == source) {
                    reached.push(source.to_owned());
                }
            }
        }
        reached
    }

    /// Whether anything here moves with no one touching it.
    pub(crate) fn move_at_rest(&self, inputs: &FunctionValues) -> bool {
        self.functions.iter().any(|function| {
            function.is_periodic() && self.exported(&function.import, inputs).0 > 0.0
        })
    }

    /// Every name this frame, for what reads them.
    /// `owner` is 0 for the opened object, else the attached one's number.
    pub(crate) fn values(
        &self,
        inputs: &FunctionInputs,
        geometry_id: u64,
        extra: &[String],
        time: f32,
        owner: u8,
    ) -> FunctionValues {
        let clock = Clock::of(time, owner);
        let mut set = FunctionValues::new();
        let names = self.imports().into_iter().map(str::to_owned).chain(extra.iter().cloned());
        for name in names {
            let value = inputs.get(&(geometry_id, name.clone())).copied().unwrap_or_else(|| self.idle(&name));
            set.insert(name, value);
        }
        // What the panel sets of a name a function makes is a script's value.
        let scripted: FunctionValues = self
            .exports_named()
            .filter_map(|export| {
                let value = inputs.get(&(geometry_id, export.to_owned()))?;
                Some((export.to_owned(), *value))
            })
            .collect();
        let mut values = set.clone();
        for export in self.exports_named() {
            let (value, _) = self.value(export, &set, &scripted, clock, 0);
            values.insert(export.to_owned(), value);
        }
        values
    }
}
