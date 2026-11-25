//! Application navigation. Exhaustive matches keep new tools visible to the compiler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tool {
    Ports,
    Cleanup,
    Overview,
    Logs,
    Processes,
    Environment,
    Docker,
    Services,
}

impl Tool {
    /// Navigation order of the rail.
    pub(super) const ALL: [Self; 8] = [
        Self::Overview,
        Self::Ports,
        Self::Processes,
        Self::Services,
        Self::Docker,
        Self::Cleanup,
        Self::Environment,
        Self::Logs,
    ];

    /// Short rail label.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Ports => "Ports",
            Self::Cleanup => "Cleanup",
            Self::Overview => "Overview",
            Self::Logs => "Logs",
            Self::Processes => "Processes",
            Self::Environment => "Environment",
            Self::Docker => "Docker",
            Self::Services => "Services",
        }
    }

    /// Descriptive title, shown as the rail tooltip.
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::Ports => "Port manager",
            Self::Cleanup => "Disk cleanup",
            Self::Overview => "System overview",
            Self::Logs => "System logs",
            Self::Processes => "Process manager",
            Self::Environment => "Developer environment",
            Self::Docker => "Docker containers",
            Self::Services => "Services",
        }
    }
}
