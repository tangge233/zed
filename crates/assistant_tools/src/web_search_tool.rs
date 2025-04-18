use std::{sync::Arc, time::Duration};

use crate::schema::json_schema_for;
use anyhow::{Context as _, Result, anyhow};
use assistant_tool::{ActionLog, Tool, ToolCard, ToolResult, ToolUseStatus};
use futures::{FutureExt, TryFutureExt};
use gpui::{
    Animation, AnimationExt, App, AppContext, Context, Entity, IntoElement, Task, Window,
    pulsating_between,
};
use language_model::{LanguageModelRequestMessage, LanguageModelToolSchemaFormat};
use project::Project;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ui::{IconName, Tooltip, prelude::*};
use web_search::WebSearchRegistry;
use zed_llm_client::WebSearchResponse;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WebSearchToolInput {
    /// The search term or question to query on the web.
    query: String,
}

pub struct WebSearchTool;

impl Tool for WebSearchTool {
    fn name(&self) -> String {
        "web_search".into()
    }

    fn needs_confirmation(&self, _: &serde_json::Value, _: &App) -> bool {
        false
    }

    fn description(&self) -> String {
        "Search the web for information using your query. Use this when you need real-time information, facts, or data that might not be in your training. Results will include snippets and links from relevant web pages.".into()
    }

    fn icon(&self) -> IconName {
        IconName::Globe
    }

    fn input_schema(&self, format: LanguageModelToolSchemaFormat) -> Result<serde_json::Value> {
        json_schema_for::<WebSearchToolInput>(format)
    }

    fn ui_text(&self, _input: &serde_json::Value) -> String {
        "Web Search".to_string()
    }

    fn run(
        self: Arc<Self>,
        input: serde_json::Value,
        _messages: &[LanguageModelRequestMessage],
        _project: Entity<Project>,
        _action_log: Entity<ActionLog>,
        cx: &mut App,
    ) -> ToolResult {
        let input = match serde_json::from_value::<WebSearchToolInput>(input) {
            Ok(input) => input,
            Err(err) => return Task::ready(Err(anyhow!(err))).into(),
        };
        let Some(provider) = WebSearchRegistry::read_global(cx).active_provider() else {
            return Task::ready(Err(anyhow!("Web search is not available."))).into();
        };

        let search_task = provider.search(input.query, cx).map_err(Arc::new).shared();
        let output = cx.background_spawn({
            let search_task = search_task.clone();
            async move {
                let response = search_task.await.map_err(|err| anyhow!(err))?;
                serde_json::to_string(&response).context("Failed to serialize search results")
            }
        });

        ToolResult {
            output,
            card: Some(cx.new(|cx| WebSearchToolCard::new(search_task, cx)).into()),
        }
    }
}

struct WebSearchToolCard {
    response: Option<Result<WebSearchResponse>>,
    _task: Task<()>,
}

impl WebSearchToolCard {
    fn new(
        search_task: impl 'static + Future<Output = Result<WebSearchResponse, Arc<anyhow::Error>>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let _task = cx.spawn(async move |this, cx| {
            let response = search_task.await.map_err(|err| anyhow!(err));
            this.update(cx, |this, cx| {
                this.response = Some(response);
                cx.notify();
            })
            .ok();
        });

        Self {
            response: None,
            _task,
        }
    }
}

impl ToolCard for WebSearchToolCard {
    fn render(
        &mut self,
        _status: &ToolUseStatus,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let header = h_flex()
            .id("tool-label-container")
            .gap_1p5()
            .max_w_full()
            .overflow_x_scroll()
            .child(
                Icon::new(IconName::Globe)
                    .size(IconSize::XSmall)
                    .color(Color::Muted),
            )
            .child(match self.response.as_ref() {
                Some(Ok(response)) => {
                    let text: SharedString = if response.citations.len() == 1 {
                        "1 result".into()
                    } else {
                        format!("{} results", response.citations.len()).into()
                    };
                    h_flex()
                        .gap_1p5()
                        .child(Label::new("Searched the Web").size(LabelSize::Small))
                        .child(
                            div()
                                .size(px(3.))
                                .rounded_full()
                                .bg(cx.theme().colors().text),
                        )
                        .child(Label::new(text).size(LabelSize::Small))
                        .into_any_element()
                }
                Some(Err(error)) => div()
                    .id("web-search-error")
                    .child(Label::new("Web Search failed").size(LabelSize::Small))
                    .tooltip(Tooltip::text(error.to_string()))
                    .into_any_element(),

                None => Label::new("Searching the Web…")
                    .size(LabelSize::Small)
                    .with_animation(
                        "web-search-label",
                        Animation::new(Duration::from_secs(2))
                            .repeat()
                            .with_easing(pulsating_between(0.6, 1.)),
                        |label, delta| label.alpha(delta),
                    )
                    .into_any_element(),
            })
            .into_any();

        let content =
            self.response.as_ref().and_then(|response| match response {
                Ok(response) => {
                    Some(
                        v_flex()
                            .ml_1p5()
                            .pl_1p5()
                            .border_l_1()
                            .border_color(cx.theme().colors().border_variant)
                            .gap_1()
                            .children(response.citations.iter().enumerate().map(
                                |(index, citation)| {
                                    let title = citation.title.clone();
                                    let url = citation.url.clone();

                                    Button::new(("citation", index), title)
                                        .label_size(LabelSize::Small)
                                        .color(Color::Muted)
                                        .icon(IconName::ArrowUpRight)
                                        .icon_size(IconSize::XSmall)
                                        .icon_position(IconPosition::End)
                                        .truncate(true)
                                        .tooltip({
                                            let url = url.clone();
                                            move |window, cx| {
                                                Tooltip::with_meta(
                                                    "Citation Link",
                                                    None,
                                                    url.clone(),
                                                    window,
                                                    cx,
                                                )
                                            }
                                        })
                                        .on_click({
                                            let url = url.clone();
                                            move |_, _, cx| cx.open_url(&url)
                                        })
                                },
                            ))
                            .into_any(),
                    )
                }
                Err(_) => None,
            });

        v_flex().my_2().gap_1().child(header).children(content)
    }
}
