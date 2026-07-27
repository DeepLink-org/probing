mod capabilities;
mod dashboard;
mod distributed;
mod explore;
mod investigate;
mod profiles;
mod training;

pub use capabilities::{
    AnalyticsPage, ClusterPage, DistributedPythonStackPage, DistributedStackPage, InferencePage,
    PerfettoPage, ProcessTimelinePage, PulsingPage, PythonPage, RlSpansPage, RlTrainPage,
    RolloutPage, SpansPage, StackPage, StackThreadPage, SystemPage,
};
pub use dashboard::DashboardPage;
pub use distributed::DistributedPage;
pub use explore::{ClassicFallbackPage, ExplorePage};
pub use investigate::InvestigatePage;
pub(crate) use investigate::InvestigateSession;
pub use profiles::{ChromeTracePage, ProfileViewPage, ProfilesPage};
pub use training::TrainingPage;
