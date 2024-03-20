pub fn init(self) -> Result<()> {
    if let Some((tracing_receiver, profiling_targets)) = self.profiling {
        if self.log_reloading {
            let subscriber = prepare_subscriber(
                &self.directives,
                Some(&profiling_targets),
                self.force_colors,
                self.detailed_output,
                |builder| enable_log_reloading!(builder),
            )?;
            let mut profiling =
                crate::ProfilingLayer::new(tracing_receiver, &profiling_targets);

            self.custom_profiler
                .into_iter()
                .for_each(|profiler| profiling.add_handler(profiler));

            //tracing::subscriber::set_global_default(subscriber.with(profiling))?;

            Ok(())
        } else {
            let subscriber = prepare_subscriber(
                &self.directives,
                Some(&profiling_targets),
                self.force_colors,
                self.detailed_output,
                |builder| builder,
            )?;
            let mut profiling =
                crate::ProfilingLayer::new(tracing_receiver, &profiling_targets);

            self.custom_profiler
                .into_iter()
                .for_each(|profiler| profiling.add_handler(profiler));

            //tracing::subscriber::set_global_default(subscriber.with(profiling))?;

            Ok(())
        }
    } else if self.log_reloading {
        let subscriber = prepare_subscriber(
            &self.directives,
            None,
            self.force_colors,
            self.detailed_output,
            |builder| enable_log_reloading!(builder),
        )?;

        //tracing::subscriber::set_global_default(subscriber)?;

        Ok(())
    } else {
        let subscriber = prepare_subscriber(
            &self.directives,
            None,
            self.force_colors,
            self.detailed_output,
            |builder| builder,
        )?;

        //tracing::subscriber::set_global_default(subscriber)?;

        Ok(())
    }
}