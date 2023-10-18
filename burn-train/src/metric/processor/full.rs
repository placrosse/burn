use super::{Event, EventProcessor, LearnerItem};
use crate::metric::store::{EventStoreClient, MetricsUpdate};
use crate::metric::{Adaptor, Metric, MetricEntry, MetricMetadata, Numeric};
use crate::renderer::{MetricState, MetricsRenderer, TrainingProgress};
use std::sync::Arc;

/// An [event processor](EventProcessor) that handles:
///   - Computing and storing metrics in an [event store](crate::metric::store::EventStore).
///   - Render metrics using a [metrics renderer](MetricsRenderer).
pub struct FullEventProcessor<T, V> {
    train: Vec<Box<dyn MetricUpdater<T>>>,
    valid: Vec<Box<dyn MetricUpdater<V>>>,
    train_numeric: Vec<Box<dyn NumericMetricUpdater<T>>>,
    valid_numeric: Vec<Box<dyn NumericMetricUpdater<V>>>,
    renderer: Box<dyn MetricsRenderer>,
    client: Arc<EventStoreClient>,
}

impl<T, V> EventProcessor for FullEventProcessor<T, V> {
    type ItemTrain = T;
    type ItemValid = V;

    fn add_event_train(&mut self, event: Event<Self::ItemTrain>) {
        match event {
            Event::ProcessedItem(item) => {
                let progress = (&item).into();
                let metadata = (&item).into();

                let update = self.update_train(&item, &metadata);

                update
                    .entries
                    .into_iter()
                    .for_each(|entry| self.renderer.update_train(MetricState::Generic(entry)));

                update
                    .entries_numeric
                    .into_iter()
                    .for_each(|(entry, value)| {
                        self.renderer
                            .update_train(MetricState::Numeric(entry, value))
                    });

                self.renderer.render_train(progress);
            }
            Event::EndEpoch(epoch) => {
                self.end_epoch_train(epoch);
            }
        }
    }

    fn add_event_valid(&mut self, event: Event<Self::ItemValid>) {
        match event {
            Event::ProcessedItem(item) => {
                let progress = (&item).into();
                let metadata = (&item).into();

                let update = self.update_valid(&item, &metadata);

                update
                    .entries
                    .into_iter()
                    .for_each(|entry| self.renderer.update_valid(MetricState::Generic(entry)));

                update
                    .entries_numeric
                    .into_iter()
                    .for_each(|(entry, value)| {
                        self.renderer
                            .update_valid(MetricState::Numeric(entry, value))
                    });

                self.renderer.render_valid(progress);
            }
            Event::EndEpoch(epoch) => {
                self.end_epoch_valid(epoch);
            }
        }
    }
}

trait NumericMetricUpdater<T>: Send + Sync {
    fn update(&mut self, item: &LearnerItem<T>, metadata: &MetricMetadata) -> (MetricEntry, f64);
    fn clear(&mut self);
}

trait MetricUpdater<T>: Send + Sync {
    fn update(&mut self, item: &LearnerItem<T>, metadata: &MetricMetadata) -> MetricEntry;
    fn clear(&mut self);
}

#[derive(new)]
struct MetricWrapper<M> {
    metric: M,
}

impl<T, M> NumericMetricUpdater<T> for MetricWrapper<M>
where
    T: 'static,
    M: Metric + Numeric + 'static,
    T: Adaptor<M::Input>,
{
    fn update(&mut self, item: &LearnerItem<T>, metadata: &MetricMetadata) -> (MetricEntry, f64) {
        let update = self.metric.update(&item.item.adapt(), metadata);
        let numeric = self.metric.value();

        (update, numeric)
    }

    fn clear(&mut self) {
        self.metric.clear()
    }
}

impl<T, M> MetricUpdater<T> for MetricWrapper<M>
where
    T: 'static,
    M: Metric + 'static,
    T: Adaptor<M::Input>,
{
    fn update(&mut self, item: &LearnerItem<T>, metadata: &MetricMetadata) -> MetricEntry {
        self.metric.update(&item.item.adapt(), metadata)
    }

    fn clear(&mut self) {
        self.metric.clear()
    }
}

pub struct FullEventProcessorBuilder<T, V> {
    train: Vec<Box<dyn MetricUpdater<T>>>,
    valid: Vec<Box<dyn MetricUpdater<V>>>,
    train_numeric: Vec<Box<dyn NumericMetricUpdater<T>>>,
    valid_numeric: Vec<Box<dyn NumericMetricUpdater<V>>>,
}

impl<T, V> Default for FullEventProcessorBuilder<T, V> {
    fn default() -> Self {
        Self {
            train: Vec::default(),
            valid: Vec::default(),
            train_numeric: Vec::default(),
            valid_numeric: Vec::default(),
        }
    }
}

impl<T, V> FullEventProcessorBuilder<T, V> {
    /// Register a training metric.
    pub(crate) fn register_metric_train<Me: Metric + 'static>(&mut self, metric: Me)
    where
        T: Adaptor<Me::Input> + 'static,
    {
        let metric = MetricWrapper::new(metric);
        self.train.push(Box::new(metric))
    }

    /// Register a validation metric.
    pub(crate) fn register_valid_metric<Me: Metric + 'static>(&mut self, metric: Me)
    where
        V: Adaptor<Me::Input> + 'static,
    {
        let metric = MetricWrapper::new(metric);
        self.valid.push(Box::new(metric))
    }

    /// Register a numeric training metric.
    pub(crate) fn register_train_metric_numeric<Me: Metric + Numeric + 'static>(
        &mut self,
        metric: Me,
    ) where
        T: Adaptor<Me::Input> + 'static,
    {
        let metric = MetricWrapper::new(metric);
        self.train_numeric.push(Box::new(metric))
    }

    /// Register a numeric validation metric.
    pub(crate) fn register_valid_metric_numeric<Me: Metric + Numeric + 'static>(
        &mut self,
        metric: Me,
    ) where
        V: Adaptor<Me::Input> + 'static,
    {
        let metric = MetricWrapper::new(metric);
        self.valid_numeric.push(Box::new(metric))
    }

    pub(crate) fn build(
        self,
        renderer: Box<dyn MetricsRenderer>,
        client: Arc<EventStoreClient>,
    ) -> FullEventProcessor<T, V> {
        FullEventProcessor {
            train: self.train,
            valid: self.valid,
            train_numeric: self.train_numeric,
            valid_numeric: self.valid_numeric,
            renderer,
            client,
        }
    }
}

impl<T, V> FullEventProcessor<T, V> {
    /// Update the training information from the training item.
    pub(crate) fn update_train(
        &mut self,
        item: &LearnerItem<T>,
        metadata: &MetricMetadata,
    ) -> MetricsUpdate {
        let mut entries = Vec::with_capacity(self.train.len());
        let mut entries_numeric = Vec::with_capacity(self.train_numeric.len());

        for metric in self.train.iter_mut() {
            let state = metric.update(item, metadata);
            entries.push(state);
        }

        for metric in self.train_numeric.iter_mut() {
            let (state, value) = metric.update(item, metadata);
            entries_numeric.push((state, value));
        }

        MetricsUpdate::new(entries, entries_numeric)
    }

    /// Update the training information from the validation item.
    pub(crate) fn update_valid(
        &mut self,
        item: &LearnerItem<V>,
        metadata: &MetricMetadata,
    ) -> MetricsUpdate {
        let mut entries = Vec::with_capacity(self.valid.len());
        let mut entries_numeric = Vec::with_capacity(self.valid_numeric.len());

        for metric in self.valid.iter_mut() {
            let state = metric.update(item, metadata);
            entries.push(state);
        }

        for metric in self.valid_numeric.iter_mut() {
            let (state, value) = metric.update(item, metadata);
            entries_numeric.push((state, value));
        }

        MetricsUpdate::new(entries, entries_numeric)
    }

    /// Signal the end of a training epoch.
    pub(crate) fn end_epoch_train(&mut self, epoch: usize) {
        for metric in self.train.iter_mut() {
            metric.clear();
        }
        for metric in self.train_numeric.iter_mut() {
            metric.clear();
        }
        self.client
            .add_event_train(crate::metric::store::Event::EndEpoch(epoch + 1));
    }

    /// Signal the end of a validation epoch.
    pub(crate) fn end_epoch_valid(&mut self, epoch: usize) {
        for metric in self.valid.iter_mut() {
            metric.clear();
        }
        for metric in self.valid_numeric.iter_mut() {
            metric.clear();
        }
        self.client
            .add_event_valid(crate::metric::store::Event::EndEpoch(epoch + 1));
    }
}

impl<T> From<&LearnerItem<T>> for TrainingProgress {
    fn from(item: &LearnerItem<T>) -> Self {
        Self {
            progress: item.progress.clone(),
            epoch: item.epoch,
            epoch_total: item.epoch_total,
            iteration: item.iteration,
        }
    }
}

impl<T> From<&LearnerItem<T>> for MetricMetadata {
    fn from(item: &LearnerItem<T>) -> Self {
        Self {
            progress: item.progress.clone(),
            epoch: item.epoch,
            epoch_total: item.epoch_total,
            iteration: item.iteration,
            lr: item.lr,
        }
    }
}
