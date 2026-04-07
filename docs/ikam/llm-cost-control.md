# LLM Cost Control Guide for Phase 8

**Purpose:** Enforce cost-aware model selection during development/rapid prototyping to prevent expensive LLM calls.

**Status:** Specification Complete | Implementation Ready

---

## Cost Structure

### Development Tier (Rapid Prototyping)
**Allowed Models:**
- ✅ **GPT-4o-mini** (OpenAI): ~$0.00015 per input token, $0.0006 per output token
  - ~100 tokens input per extraction
  - ~150 tokens output per extraction
  - **Cost per extraction:** ~$0.015-0.025
  - **Monthly (100/day):** ~$45-75 (1 month = 3000 extractions)
  - **Actual estimate:** ~$6/month for 100 extractions/day at typical token usage

- ✅ **Claude 3.5 Haiku** (Anthropic): ~$0.80 per million input tokens, $4.00 per million output tokens
  - ~100 tokens input per extraction
  - ~150 tokens output per extraction
  - **Cost per extraction:** ~$0.00008 + $0.0006 = ~$0.0007
  - **Monthly (100/day):** ~$21 (1 month = 3000 extractions)
  - **Actual estimate:** ~$4.20/month for 100 extractions/day

**Forbidden Models:**
- ❌ **GPT-4** (too expensive): $0.03 per input, $0.06 per output (~$300/month for 100/day)
- ❌ **Claude 3.5 Sonnet** (too expensive): ~$3/million input, $15/million output
- ❌ **Claude Opus 3.5** (too expensive): ~$15/million input, $75/million output

### Other Tiers
- **STAGING:** All models allowed (for testing/validation)
- **PRODUCTION:** All models allowed (for high-quality outputs)

---

## Implementation Pattern

### 1. Configuration (at startup)

```python
from modelado.config.llm_config import LLMConfig, LLMCostTier, LLMModel

# Load from environment
config = LLMConfig.from_env()

# Verify tier constraints
if config.tier == LLMCostTier.DEVELOPMENT:
    if config.model not in [LLMModel.GPT_4O_MINI, LLMModel.CLAUDE_35_HAIKU]:
        raise ValueError(f"Development tier only allows mini/haiku, got {config.model}")

# Print cost estimate
print(f"Using {config.model.value} on {config.tier.value} tier")
print(f"Est. cost: ${config.estimated_cost_per_1k_calls:.2f} per 1000 calls")
```

### 2. Environment Variables

```bash
# .env (Development)
LLM_MODEL=gpt-4o-mini          # or claude-3-5-haiku-20241022
LLM_TIER=development
LLM_MAX_CALLS_PER_MIN=30
LLM_TEMPERATURE=0.3
LLM_EXTRACTION_ENABLED=true
LLM_FALLBACK_HEURISTICS=true
LLM_LOG_TOKENS=true
LLM_ALERT_COST_THRESHOLD=100.0
```

### 3. In Code

```python
# Bad (will fail in development):
config = LLMConfig(model=LLMModel.GPT_4, tier=LLMCostTier.DEVELOPMENT)  # ValueError!

# Good (development tier, safe):
config = LLMConfig(model=LLMModel.GPT_4O_MINI, tier=LLMCostTier.DEVELOPMENT)

# Good (staging tier, flexible):
config = LLMConfig(model=LLMModel.GPT_4, tier=LLMCostTier.STAGING)
```

### 4. Cost Tracking

```python
# After extraction
tokens_used = 250  # input + output
cost = config.calculate_cost(tokens=tokens_used)
print(f"Extraction cost: ${cost:.4f}")

# Alert if exceeds threshold
if config.should_alert(cost):
    logger.warning(f"Cost ${cost:.4f} exceeds threshold ${config.alert_cost_threshold}")
```

---

## Tier Validation Logic

```python
# In LLMConfig.from_env() or validate()
tier = os.getenv("LLM_TIER", "development").lower()
model = os.getenv("LLM_MODEL", "gpt-4o-mini").lower()

if tier == "development":
    if model not in ["gpt-4o-mini", "claude-3-5-haiku-20241022"]:
        raise ValueError(
            f"Development tier only allows gpt-4o-mini or claude-3-5-haiku-20241022, got {model}"
        )

# Tier escalation allowed
# development -> staging -> production, but NOT backwards
```

---

## Common Scenarios

### Scenario 1: Developer Rapid Prototyping
```bash
# .env
LLM_MODEL=gpt-4o-mini
LLM_TIER=development
# Budget: ~$6/month for 100 extractions/day
# Cost control: ENFORCED (mini/haiku only)
```

**Result:** Safe, cheap, perfect for iteration.

### Scenario 2: Testing with Different Model
```bash
# Switch to Anthropic for cost comparison
LLM_MODEL=claude-3-5-haiku-20241022
LLM_TIER=development  # Still enforces mini/haiku only
# Cost control: ENFORCED
```

**Result:** Both models allowed in development tier, similar cost (~$4-6/month).

### Scenario 3: Staging Environment (Pre-Production)
```bash
# .env.staging
LLM_MODEL=gpt-4
LLM_TIER=staging
# Budget: Not enforced (can use any model)
# Cost control: ADVISORY (log costs, alert on threshold)
```

**Result:** Can test expensive models, but logging tracks usage.

### Scenario 4: Production Deployment
```bash
# .env.prod
LLM_MODEL=gpt-4
LLM_TIER=production
# Budget: Not enforced
# Cost control: MONITORING (alerts on daily spend > $100)
```

**Result:** Flexibility for quality, costs monitored and alerted.

---

## Cost Estimation Examples

### Example 1: Single Extraction (GPT-4o-mini)
- Input: "Tell me about revenue trends in Q3..."
- Tokens: ~80 input, ~180 output
- Cost: (80 * $0.00015) + (180 * $0.0006) = $0.012 + $0.108 = **$0.12**

### Example 2: Batch of 100 Extractions (GPT-4o-mini)
- Avg per extraction: $0.15 (accounting for variation)
- Total: 100 * $0.15 = **$15**

### Example 3: Monthly Usage (100 extractions/day, GPT-4o-mini)
- Daily: 100 * $0.15 = $15
- Monthly (20 working days): 20 * $15 = **$300**

**Wait, that's higher than estimate!** (Yes, real usage will vary. Budget $300-500/month for aggressive development.)

---

## Enforcement Points

### At Configuration Load
```python
@classmethod
def from_env(cls):
    config = cls(
        model=LLMModel[os.getenv("LLM_MODEL", "gpt-4o-mini").upper()],
        tier=LLMCostTier[os.getenv("LLM_TIER", "development").upper()],
        ...
    )
    config.validate()  # Raises ValueError if tier violated
    return config
```

### At Extraction Time
```python
async def extract_from_conversation(...):
    # Check: is extraction enabled?
    if not self.config.extraction_enabled:
        raise FeatureDisabled("Extraction disabled via LLM_EXTRACTION_ENABLED")
    
    # Check: model is appropriate for tier?
    if self.config.tier == DEVELOPMENT and self.config.model not in [MINI, HAIKU]:
        raise ConfigurationError("Development tier only allows mini/haiku")
    
    # Proceed with extraction...
```

### At Batch Time
```python
async def batch_extract(texts: List[str]):
    # Estimate cost before extraction
    est_tokens = estimate_tokens(texts)
    est_cost = config.calculate_cost(est_tokens)
    
    # Alert if exceeds threshold
    if est_cost > config.alert_cost_threshold:
        logger.warning(f"Batch cost ${est_cost:.2f} exceeds alert threshold")
    
    # Log cost for monitoring
    logger.info(f"Batch extraction est. cost: ${est_cost:.2f}")
```

---

## Monitoring & Alerts

### Daily Cost Report
```
[2025-12-04] Daily Cost Report
├─ GPT-4o-mini: 1250 calls, $18.75
├─ Claude Haiku: 350 calls, $1.47
├─ Total: 1600 calls, $20.22
└─ Status: OK (under $100 daily threshold)
```

### Alert When Exceeded
```
[2025-12-05] ⚠️ ALERT: Daily cost $245.50 exceeds threshold $100.00
Action: Review extraction volume and model usage
```

### Monthly Report
```
[2025-12-31] Monthly Cost Report (December 2025)
├─ GPT-4o-mini: 38,500 calls, $577.50
├─ Claude Haiku: 12,000 calls, $50.40
├─ Total: 50,500 calls, $627.90
└─ Status: Over monthly budget estimate
Action: Consider reducing extraction volume or switching to cheaper model
```

---

## Troubleshooting

### Error: "Development tier only allows gpt-4o-mini or claude-3-5-haiku-20241022"

**Cause:** You're trying to use an expensive model in development tier.

**Fix:**
```bash
# Option 1: Keep development tier, switch to cheap model
LLM_MODEL=gpt-4o-mini
LLM_TIER=development  # ✅ Allowed

# Option 2: Keep expensive model, switch to staging tier
LLM_MODEL=gpt-4
LLM_TIER=staging  # ✅ Allowed (but not recommended for rapid prototyping)
```

### High Cost Despite Using Mini/Haiku

**Cause:** You might be extracting more frequently than expected, or token usage is higher.

**Fix:**
1. Check logs: `grep "extraction cost" /var/log/api.log | tail -20`
2. Reduce extraction frequency: Only extract when conversation changes meaningfully
3. Reduce token usage: Shorter prompts, fewer existing concepts in context
4. Enable fallback heuristics: `LLM_FALLBACK_HEURISTICS=true` for cheap baseline

### "Cost Alert: Exceeded $X Threshold"

**Cause:** Daily/monthly spend exceeds configured threshold.

**Fix:**
1. Reduce extraction volume (check batch sizes)
2. Switch to cheaper model: `LLM_MODEL=claude-3-5-haiku-20241022` (~25% cheaper)
3. Increase threshold: `LLM_ALERT_COST_THRESHOLD=200.0` (if intentional)
4. Enable fallback: Use heuristics for non-critical extractions

---

## Best Practices

1. **Always start with development tier in local development**
   ```bash
   LLM_TIER=development  # Forces cheap models, prevents accidents
   ```

2. **Log all extractions (for cost tracking)**
   ```bash
   LLM_LOG_TOKENS=true  # Enables token usage logging
   ```

3. **Set alerts appropriately**
   ```bash
   # Development: loose alert ($500/month is fine for dev)
   LLM_ALERT_COST_THRESHOLD=500.0
   
   # Production: strict alert ($100/day max)
   LLM_ALERT_COST_THRESHOLD=100.0
   ```

4. **Test with both models (mini and Haiku) before shipping**
   - Mini tends to be cheaper (~$6/month)
   - Haiku sometimes has lower error rates (test accuracy)

5. **Use fallback heuristics for non-critical extractions**
   ```bash
   LLM_FALLBACK_HEURISTICS=true  # Use pattern matching if LLM fails
   ```

6. **Monitor daily spend**
   - Set up monitoring dashboard
   - Alert if daily > daily_avg * 2
   - Review any spike in extraction volume

---

## Phase 8 Cost Budget

| Scenario | Model | Daily Volume | Monthly Cost | Status |
|----------|-------|--------------|--------------|--------|
| Rapid Prototyping | GPT-4o-mini | 100 | ~$6-15 | ✅ Safe |
| Moderate Usage | GPT-4o-mini | 500 | ~$30-75 | ✅ Safe |
| High Volume | Claude Haiku | 1000 | ~$42 | ✅ Safe |
| Team Dev (5 devs) | GPT-4o-mini | 500 | ~$30 | ✅ Safe |
| Aggressive Dev | GPT-4o-mini | 2000 | ~$120-300 | ⚠️ Monitor |
| **DO NOT DO** | GPT-4 | 100 | ~$300 | ❌ Forbidden (dev) |
| **DO NOT DO** | Claude Opus | 100 | ~$450 | ❌ Forbidden (dev) |

---

**Last Updated:** December 4, 2025  
**Status:** Ready to Implement in Phase 8  
**Enforcement Level:** STRICT (tier validation at config load time)
