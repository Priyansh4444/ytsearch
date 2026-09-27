#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct EngagementValues {
    pub(super) views: Option<u64>,
    pub(super) likes: Option<u64>,
    pub(super) comments: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct EngagementContext {
    max_views: u64,
    max_likes: u64,
    max_comments: u64,
}

impl EngagementContext {
    pub(super) fn from_generation(values: impl IntoIterator<Item = EngagementValues>) -> Self {
        values
            .into_iter()
            .fold(Self::default(), |mut maxima, value| {
                maxima.max_views = maxima.max_views.max(value.views.unwrap_or_default());
                maxima.max_likes = maxima.max_likes.max(value.likes.unwrap_or_default());
                maxima.max_comments = maxima.max_comments.max(value.comments.unwrap_or_default());
                maxima
            })
    }

    pub(super) fn normalized(self, value: EngagementValues) -> f32 {
        let views = normalize(value.views.unwrap_or_default(), self.max_views);
        let likes = normalize(value.likes.unwrap_or_default(), self.max_likes);
        let comments = normalize(value.comments.unwrap_or_default(), self.max_comments);
        let normalized = 0.50_f32 * views + 0.35_f32 * likes + 0.15_f32 * comments;
        assert!(normalized.is_finite(), "engagement score must be finite");
        normalized
    }

    pub(super) fn blended(self, raw_relevance: f32, value: EngagementValues) -> f32 {
        assert!(raw_relevance.is_finite(), "raw relevance must be finite");
        let normalized = self.normalized(value);
        let blend = raw_relevance.max(1.0_f32) * 0.20_f32 * normalized;
        let blended = raw_relevance + blend;
        assert!(blended.is_finite(), "blended relevance must be finite");
        blended
    }
}

fn normalize(value: u64, maximum: u64) -> f32 {
    if maximum == 0 {
        return 0.0;
    }
    (1.0_f32 + value as f32).ln() / (1.0_f32 + maximum as f32).ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_maxima_missing_zero_and_blend_are_bit_exact() {
        let context = EngagementContext::from_generation([
            EngagementValues {
                views: Some(100),
                likes: Some(10),
                comments: Some(2),
            },
            EngagementValues {
                views: None,
                likes: Some(0),
                comments: None,
            },
        ]);
        let maximum = EngagementValues {
            views: Some(100),
            likes: Some(10),
            comments: Some(2),
        };
        assert_eq!(context.normalized(maximum).to_bits(), 0x3f80_0000);
        assert_eq!(context.normalized(EngagementValues::default()).to_bits(), 0);
        assert_eq!(context.blended(2.0, maximum).to_bits(), 0x4019_999a);
    }
}
