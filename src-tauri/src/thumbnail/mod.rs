mod coordinator;
mod model;
mod resolver;

pub use coordinator::{
    ThumbnailCoordinator, ThumbnailCoordinatorConfig, ThumbnailCoordinatorError,
    ThumbnailReceiveTimeout, ThumbnailRequestHandle,
};
pub use model::{
    ResolvedThumbnail, ThumbnailCacheClearDto, ThumbnailCacheStatus, ThumbnailCompletionEventDto,
    ThumbnailCompletionOutcomeDto, ThumbnailConsumer, ThumbnailDeliveryDto, ThumbnailFailureCode,
    ThumbnailFailureDto, ThumbnailInvalidationDto, ThumbnailKey, ThumbnailKeyError,
    ThumbnailPriority, ThumbnailRequestDto, ThumbnailRequestTokenDto, ThumbnailResult,
    ThumbnailRuntimeConfigDto, ThumbnailWorkerStatsDto,
};
pub use resolver::{
    CancellationToken, FixtureThumbnailResolver, ThumbnailResolveError, ThumbnailResolver,
};

#[cfg(test)]
mod tests;
