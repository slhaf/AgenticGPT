use crate::config_templates::{OptionalSection, RuntimeMode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConfigPage {
    Basic,
    Connection,
    OptionalCenter,
    Optional(OptionalSection),
    Review,
    SystemError,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReturnTarget {
    MainFlow,
    Review,
}

pub(crate) struct Navigation {
    mode: RuntimeMode,
    flow: Vec<ConfigPage>,
    index: usize,
    #[allow(dead_code)]
    return_target: ReturnTarget,
}

impl Navigation {
    #[allow(dead_code)]
    pub(crate) fn new(mode: RuntimeMode) -> Self {
        Self {
            mode,
            flow: flow_for(mode),
            index: 0,
            return_target: ReturnTarget::MainFlow,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn mode(&self) -> RuntimeMode {
        self.mode
    }

    #[allow(dead_code)]
    pub(crate) fn flow(&self) -> Vec<ConfigPage> {
        self.flow.clone()
    }

    pub(crate) fn current(&self) -> ConfigPage {
        self.flow[self.index]
    }

    pub(crate) fn progress(&self) -> (usize, usize) {
        (self.index + 1, self.flow.len())
    }

    pub(crate) fn advance(&mut self) -> bool {
        if self.index + 1 >= self.flow.len() {
            false
        } else {
            self.index += 1;
            true
        }
    }

    pub(crate) fn back(&mut self) -> bool {
        if self.index == 0 {
            false
        } else {
            self.index -= 1;
            true
        }
    }

    pub(crate) fn set_mode(&mut self, mode: RuntimeMode) {
        let current = self.current();
        self.mode = mode;
        self.flow = flow_for(mode);
        self.index = self
            .flow
            .iter()
            .position(|page| *page == current)
            .unwrap_or(0);
    }

    #[allow(dead_code)]
    pub(crate) fn set_return_target(&mut self, target: ReturnTarget) {
        self.return_target = target;
    }

    #[allow(dead_code)]
    pub(crate) fn return_target(&self) -> ReturnTarget {
        self.return_target
    }

    pub(crate) fn go_to(&mut self, page: ConfigPage) -> bool {
        if let Some(index) = self.flow.iter().position(|candidate| *candidate == page) {
            self.index = index;
            true
        } else {
            false
        }
    }
}

fn flow_for(mode: RuntimeMode) -> Vec<ConfigPage> {
    let mut flow = vec![ConfigPage::Basic];
    if mode != RuntimeMode::Local {
        flow.push(ConfigPage::Connection);
    }
    flow.extend([ConfigPage::OptionalCenter, ConfigPage::Review]);
    flow
}
