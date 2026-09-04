pub mod ai_providers;
pub mod backtest;
pub mod binance;
pub mod cloud_relay;
pub mod codex;
pub mod crypto_contracts;
pub mod crypto_market;
pub mod data_pipeline;
pub mod data_quality;
pub mod decision;
pub mod engine_runtime;
pub mod execution_algorithms;
pub mod execution_control;
pub mod experiments;
pub mod external_links;
pub mod forecast;
pub mod forecast_runtime;
pub mod futures_paper;
pub mod github_auth;
pub mod governance;
pub mod kis_paper;
pub mod local_security;
pub mod manual_orders;
pub mod market_aggregation;
pub mod market_data;
pub mod meeting_handoff;
pub mod ml_pipeline;
pub mod ml_worker_runner;
pub mod official_kr_data;
pub mod operational_controls;
pub mod operational_readiness;
pub mod operations;
pub mod orchestration;
pub mod paper_account;
pub mod paper_trading;
pub mod pattern_probability;
pub mod performance;
pub mod persistence;
pub mod pit_dataset;
pub mod pit_providers;
pub mod publicity;
pub mod quant_risk;
pub mod reference;
pub mod remote_control;
pub mod research;
pub mod risk_policy;
pub mod runtime_ops;
pub mod screening;
pub mod sec_fundamentals;
pub mod simulation;
pub mod social_auth;
pub mod strategy_deployment;
pub mod strategy_plugins;
pub mod strategy_protection;
pub mod telegram;
pub mod toss_stream;
pub mod trading;
pub mod workspace_identity;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(codex::CodexBridge::default())
        .manage(ai_providers::AiProviderBridge::default())
        .manage(binance::BinanceBridge::default())
        .manage(crypto_market::CryptoMarketBridge::default())
        .manage(market_data::MarketDataBridge::default())
        .manage(official_kr_data::OfficialKrDataBridge::new())
        .manage(reference::ReferenceFetcher::default())
        .manage(sec_fundamentals::SecFundamentalsBridge::default())
        .manage(telegram::TelegramBridge::default())
        .manage(workspace_identity::WorkspaceIdentityBridge::default())
        .manage(operations::ShadowEngineRuntime::default())
        .manage(market_aggregation::MarketAggregationBridge::default())
        .manage(toss_stream::TossMarketStreamBridge::default())
        .manage(pit_providers::PitProviderBridge::default())
        .manage(pit_providers::PitCollectionRuntime::default())
        .setup(|app| {
            use tauri::Manager;

            let app_data_dir = app.path().app_data_dir()?;
            local_security::harden_app_data(&app_data_dir).map_err(std::io::Error::other)?;
            let headless_shadow_soak = if operational_readiness::headless_shadow_soak_requested() {
                if let Some(window) = app.get_webview_window("main") {
                    window.hide()?;
                }
                match operational_readiness::acquire_headless_shadow_soak(&app_data_dir)
                    .map_err(std::io::Error::other)?
                {
                    Some(guard) => Some(guard),
                    None => {
                        app.handle().exit(0);
                        return Ok(());
                    }
                }
            } else {
                None
            };
            let database_path = app_data_dir.join("investa.sqlite3");
            let persistence = persistence::PersistenceBridge::open(&database_path)
                .map_err(std::io::Error::other)?;
            runtime_ops::mark_runtime_reconciliation_required(
                &persistence,
                persistence::now_ms().map_err(std::io::Error::other)?,
            )
            .map_err(std::io::Error::other)?;
            app.manage(persistence);
            cloud_relay::start_polling(app.handle().clone());
            operations::start_shadow_engine(app.handle().clone());
            pit_providers::start_collection_scheduler(app.handle().clone());
            if let Some(guard) = headless_shadow_soak {
                operational_readiness::start_headless_shadow_soak(
                    app.handle().clone(),
                    app_data_dir,
                    guard,
                )
                .map_err(std::io::Error::other)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ai_providers::ai_provider_statuses,
            ai_providers::ai_provider_save_config,
            ai_providers::ai_provider_delete_config,
            ai_providers::ai_provider_run_analysis,
            ai_providers::ai_provider_run_role_report,
            ai_providers::ai_provider_run_department_report,
            ai_providers::ai_provider_cancel_job,
            official_kr_data::official_kr_data_status,
            official_kr_data::official_kr_data_save_config,
            official_kr_data::opendart_disclosures,
            official_kr_data::opendart_company_disclosures,
            official_kr_data::opendart_company_split_decisions,
            official_kr_data::naver_news_search,
            codex::codex_status,
            codex::codex_start_turn,
            codex::codex_cancel_turn,
            codex::codex_usage_status,
            binance::binance_connection_status,
            binance::binance_public_snapshot,
            binance::binance_perpetual_analysis_snapshot,
            binance::binance_save_credentials,
            binance::binance_account_snapshot,
            binance::binance_delete_credentials,
            crypto_market::upbit_chart_snapshot,
            crypto_market::upbit_analysis_snapshot,
            crypto_market::upbit_market_quote,
            crypto_market::upbit_run_research_backtest,
            crypto_market::upbit_execute_paper_market_order,
            crypto_market::upbit_connection_status,
            crypto_market::upbit_save_credentials,
            crypto_market::upbit_delete_credentials,
            crypto_market::upbit_account_snapshot,
            external_links::open_official_external_page,
            engine_runtime::engine_run_execute,
            engine_runtime::engine_run_history,
            engine_runtime::engine_run_detail,
            engine_runtime::engine_run_cancel,
            engine_runtime::engine_run_restart,
            engine_runtime::engine_runtime_overview,
            data_pipeline::data_normalize_preview,
            experiments::backtest_experiment_clone_run,
            experiments::backtest_experiment_walk_forward,
            experiments::backtest_experiment_walk_forward_latest,
            experiments::backtest_experiment_walk_forward_history,
            experiments::backtest_experiment_bias_audit,
            futures_paper::futures_paper_status,
            futures_paper::futures_paper_open,
            futures_paper::futures_paper_mark,
            futures_paper::futures_paper_close,
            futures_paper::futures_lifecycle_record,
            futures_paper::futures_lifecycle_history,
            forecast_runtime::forecast_dataset_audit_save,
            forecast_runtime::probability_forecast_save,
            forecast_runtime::forecast_calibration_save,
            forecast_runtime::probability_forecast_history,
            ml_pipeline::ml_dataset_manifest_create,
            ml_pipeline::ml_dataset_shard_set_create,
            ml_pipeline::ml_dataset_shard_set_detail,
            ml_pipeline::ml_dataset_shard_set_history,
            ml_pipeline::ml_training_job_prepare,
            ml_pipeline::ml_training_job_bundle,
            ml_pipeline::ml_training_job_complete,
            ml_pipeline::ml_pipeline_history,
            pit_dataset::pit_collection_plan_create,
            pit_dataset::pit_dataset_build_preview,
            pit_dataset::pit_dataset_build_commit,
            pit_dataset::pit_stored_dataset_build_preview,
            pit_dataset::pit_stored_dataset_build_commit,
            pit_providers::pit_provider_page_fetch,
            pit_providers::pit_provider_page_fetch_store,
            pit_providers::pit_provider_stored_range,
            pit_providers::pit_collection_job_create,
            pit_providers::pit_collection_job_run,
            pit_providers::pit_collection_job_cancel,
            pit_providers::pit_collection_job_detail,
            pit_providers::pit_collection_job_history,
            ml_worker_runner::ml_training_job_run,
            github_auth::github_session,
            github_auth::github_login_start,
            github_auth::github_link_current_session,
            social_auth::social_auth_status,
            social_auth::social_auth_save_google_client,
            social_auth::social_auth_delete_google_client,
            social_auth::google_login,
            social_auth::google_link_account,
            workspace_identity::workspace_identity_status,
            workspace_identity::workspace_identity_lifecycle_policy,
            workspace_identity::workspace_identity_unlink_provider,
            workspace_identity::workspace_identity_logout,
            market_data::market_indices_snapshot,
            market_data::toss_market_calendars,
            market_data::toss_chart_snapshot,
            market_data::toss_analysis_snapshot,
            market_data::toss_market_quote,
            market_data::toss_market_screener,
            market_data::toss_search_stocks,
            market_data::toss_run_research_backtest,
            market_data::toss_connection_status,
            market_data::toss_account_snapshot,
            market_data::toss_execute_paper_market_order,
            market_data::toss_save_credentials,
            market_data::toss_delete_credentials,
            market_aggregation::market_stream_tick_ingest,
            market_aggregation::market_stream_gap_backfill,
            market_aggregation::market_stream_aggregation_flush,
            market_aggregation::market_stream_aggregation_status,
            toss_stream::toss_market_stream_start,
            toss_stream::toss_market_stream_stop,
            toss_stream::toss_market_stream_status,
            manual_orders::manual_paper_limit_order_submit,
            manual_orders::manual_paper_orders,
            manual_orders::manual_paper_order_cancel,
            execution_algorithms::internal_execution_create,
            execution_algorithms::internal_execution_fill,
            execution_algorithms::internal_execution_reprice,
            execution_algorithms::internal_execution_cancel,
            execution_algorithms::internal_execution_expire,
            execution_algorithms::internal_execution_get,
            orchestration::agenda_execution_policy,
            operations::paper_order_candidate_create,
            operations::paper_order_candidates,
            operations::paper_order_candidate_approve,
            operations::paper_order_candidate_reject,
            operations::operations_recover,
            operations::shadow_runtime_status,
            operations::shadow_watch_arm,
            operations::shadow_watch_stop,
            operations::shadow_engine_tick,
            operations::meeting_workflow_start,
            operations::meeting_workflow_checkpoint,
            operations::meeting_workflow_interrupted,
            operations::meeting_workflow_resume,
            operations::meeting_workflow_dismiss,
            meeting_handoff::meeting_paper_handoff_prepare,
            meeting_handoff::meeting_paper_handoff_finalize,
            meeting_handoff::meeting_paper_handoff_history,
            meeting_handoff::meeting_paper_golden_path_audit,
            operational_readiness::workspace_preferences_get,
            operational_readiness::workspace_preferences_save,
            operational_readiness::market_order_normalize,
            operational_readiness::provider_retry_decision,
            operational_readiness::evidence_synthesis_preview,
            operational_readiness::paper_exit_reason_resolve,
            operational_readiness::shadow_soak_audit,
            operational_readiness::shadow_soak_audit_save,
            operational_readiness::shadow_soak_sample,
            operational_readiness::operations_dashboard_snapshot,
            operational_readiness::cloud_soak_report_snapshot,
            operational_controls::paper_risk_monitor_evaluate,
            operational_controls::trade_quality_analyze,
            operational_controls::crypto_risk_policy_change_save,
            operational_controls::crypto_risk_policy_history,
            operational_controls::operations_drill_execute,
            operational_controls::operations_drill_history,
            runtime_ops::engine_order_candidate_create,
            runtime_ops::engine_order_candidates,
            runtime_ops::engine_order_candidate_approve,
            runtime_ops::engine_order_candidate_reject,
            runtime_ops::operations_runtime_reconcile,
            runtime_ops::runtime_reconciliation_status,
            runtime_ops::operational_alerts,
            runtime_ops::operational_alert_acknowledge,
            runtime_ops::audit_event_history,
            runtime_ops::audit_event_export,
            runtime_ops::provider_health_record,
            runtime_ops::provider_health_report,
            runtime_ops::operations_health_refresh,
            runtime_ops::local_backup_create,
            runtime_ops::local_backup_inspect,
            runtime_ops::local_backup_rehearse,
            runtime_ops::local_backup_inventory,
            runtime_ops::local_recovery_evidence_export,
            paper_trading::paper_account_status,
            paper_trading::paper_accounts_status,
            paper_trading::toss_order_adapter_status,
            kis_paper::kis_paper_config_status,
            kis_paper::kis_futures_connection_status,
            kis_paper::kis_futures_analysis_snapshot,
            kis_paper::kis_paper_config_save,
            kis_paper::kis_paper_config_delete,
            kis_paper::kis_paper_account_snapshot,
            kis_paper::kis_paper_order_submit,
            kis_paper::kis_paper_order_cancel,
            kis_paper::kis_paper_orders_today,
            kis_paper::kis_paper_reconcile,
            persistence::persistence_status,
            persistence::research_run_history,
            persistence::research_run_detail,
            persistence::analysis_record_history,
            persistence::analysis_record_detail,
            persistence::analysis_note_save,
            persistence::paper_ledger_history,
            persistence::backtest_replay_history,
            publicity::publicity_evidence_pack_preview,
            publicity::publicity_draft_preview,
            publicity::publicity_media_review,
            publicity::publicity_manual_package_export,
            publicity::publicity_article_draft_save,
            publicity::publicity_article_approve,
            publicity::publicity_article_reject,
            publicity::publicity_article_latest,
            quant_risk::portfolio_risk_analyze,
            quant_risk::portfolio_risk_snapshot_save,
            quant_risk::portfolio_risk_snapshot_history,
            quant_risk::portfolio_risk_from_ledger,
            risk_policy::risk_policy_evaluate,
            risk_policy::risk_policy_save_recommendation,
            risk_policy::risk_policy_approve,
            risk_policy::risk_policy_status,
            sec_fundamentals::sec_connection_status,
            sec_fundamentals::sec_connection_probe,
            sec_fundamentals::sec_save_contact,
            sec_fundamentals::sec_delete_contact,
            strategy_protection::strategy_protection_evaluate,
            strategy_protection::strategy_protection_history,
            strategy_protection::strategy_protection_alerts_sync,
            strategy_plugins::strategy_plugin_catalog,
            strategy_plugins::strategy_plugin_validate,
            strategy_plugins::strategy_cadence_catalog,
            strategy_plugins::strategy_cadence_validate,
            strategy_deployment::strategy_deployment_candidate_create,
            strategy_deployment::strategy_deployment_canary_approve,
            strategy_deployment::strategy_deployment_canary_observe,
            strategy_deployment::strategy_deployment_paper_approve,
            strategy_deployment::strategy_deployment_rollback,
            strategy_deployment::strategy_deployment_reject,
            strategy_deployment::strategy_deployment_history,
            telegram::telegram_connection_status,
            telegram::telegram_save_credentials,
            telegram::telegram_login_start,
            telegram::telegram_login_code,
            telegram::telegram_login_password,
            telegram::telegram_channels,
            telegram::telegram_select_channels,
            telegram::telegram_sync_selected,
            telegram::telegram_evidence_snapshot,
            telegram::telegram_delete_connection,
            remote_control::remote_control_status,
            remote_control::remote_control_policy_save,
            remote_control::remote_control_instruction_ingest,
            remote_control::remote_control_jobs,
            remote_control::remote_control_job_approve,
            remote_control::remote_control_job_reject,
            remote_control::remote_control_job_cancel,
            cloud_relay::cloud_relay_status,
            cloud_relay::cloud_relay_save_configuration,
            cloud_relay::cloud_relay_delete_configuration,
            cloud_relay::cloud_relay_pull_job,
            cloud_relay::cloud_relay_submit_result
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
