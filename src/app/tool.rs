//! Application navigation. Exhaustive matches keep new tools visible to the compiler.
use crate::ui::Glyph;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tool {
    Ports,
    Cleanup,
    Overview,
    Computer,
    Logs,
    Processes,
    Environment,
    Docker,
    Services,
}

impl Tool {
    /// Navigation order of the rail.
    pub(super) const ALL: [Self; 9] = [
        Self::Overview,
        Self::Computer,
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
            Self::Computer => "Computer",
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
            Self::Computer => "Hardware inventory",
            Self::Logs => "System logs",
            Self::Processes => "Process manager",
            Self::Environment => "Developer environment",
            Self::Docker => "Docker containers",
            Self::Services => "Services",
        }
    }

    /// Line icon drawn beside the rail label.
    pub(super) fn glyph(self) -> Glyph {
        match self {
            Self::Ports => Glyph::Ports,
            Self::Cleanup => Glyph::Cleanup,
            Self::Overview => Glyph::Overview,
            Self::Computer => Glyph::Computer,
            Self::Logs => Glyph::Logs,
            Self::Processes => Glyph::Processes,
            Self::Environment => Glyph::Environment,
            Self::Docker => Glyph::Docker,
            Self::Services => Glyph::Services,
        }
    }
}
