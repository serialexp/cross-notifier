// ABOUTME: Settings types and conversion functions shared between settings implementations.
// ABOUTME: Defines state structures for editing notification daemon configuration.

package main

// SettingsResult holds the outcome of the settings window.
type SettingsResult struct {
	Config    *Config
	Cancelled bool
}

// settingsState holds the editable state for the settings window.
type settingsState struct {
	name             string
	servers          []serverEntry
	rulesEnabled     bool
	rules            []notificationRuleEntry
	autostartEnabled bool
	autostartInitial bool // track initial state to detect changes
	debugFontMetrics bool
}

// serverEntry holds editable fields for a single server.
type serverEntry struct {
	url       string
	secret    string
	label     string
	connected bool // connection status
}

// notificationRuleEntry holds editable fields for a single notification rule.
type notificationRuleEntry struct {
	serverIdx int32  // 0 = Any, 1+ = server index + 1
	source    string // source filter (empty = any)
	statusIdx int32  // 0 = Any, 1+ = status index + 1
	pattern   string // regex pattern
	soundIdx  int32  // 0 = None, 1+ = built-in sounds
	actionIdx int32  // 0 = Normal, 1 = Silent, 2 = Dismiss
}

// Status options for notification rules
var statusOptions = []string{"Any Status", "info", "success", "warning", "error"}

// Sound options (No Sound + built-in sounds)
var soundOptions = append([]string{"No Sound"}, BuiltinSounds...)

// Action options for notification rules
var actionOptions = []string{"Normal", "Silent", "Dismiss"}

// stateToConfig converts the settings state to a Config.
func stateToConfig(state *settingsState) *Config {
	cfg := &Config{
		Name:             state.name,
		DebugFontMetrics: state.debugFontMetrics,
	}
	for _, s := range state.servers {
		if s.url != "" {
			cfg.Servers = append(cfg.Servers, Server{
				URL:    s.url,
				Secret: s.secret,
				Label:  s.label,
			})
		}
	}
	// Convert rules config
	cfg.Rules.Enabled = state.rulesEnabled
	for _, entry := range state.rules {
		rule := entryToRule(entry, state.servers)
		cfg.Rules.Rules = append(cfg.Rules.Rules, rule)
	}
	return cfg
}

// ruleToEntry converts a NotificationRule to editable entry state.
func ruleToEntry(rule NotificationRule, servers []Server) notificationRuleEntry {
	entry := notificationRuleEntry{
		pattern: rule.Pattern,
		source:  rule.Source,
	}

	// Convert action to index
	switch rule.EffectiveAction() {
	case RuleActionNormal:
		entry.actionIdx = 0
	case RuleActionSilent:
		entry.actionIdx = 1
	case RuleActionDismiss:
		entry.actionIdx = 2
	}

	// Find server index (0 = Any)
	if rule.Server == "" {
		entry.serverIdx = 0
	} else {
		for i, s := range servers {
			label := s.Label
			if label == "" {
				label = s.URL
			}
			if label == rule.Server {
				entry.serverIdx = int32(i + 1)
				break
			}
		}
	}

	// Find status index (0 = Any)
	if rule.Status == "" {
		entry.statusIdx = 0
	} else {
		for i, s := range statusOptions[1:] { // skip "Any"
			if s == rule.Status {
				entry.statusIdx = int32(i + 1)
				break
			}
		}
	}

	// Find sound index
	if rule.Sound == "none" || rule.Sound == "" {
		entry.soundIdx = 0
	} else if IsBuiltinSound(rule.Sound) {
		for i, s := range BuiltinSounds {
			if s == rule.Sound {
				entry.soundIdx = int32(i + 1) // +1 because 0 is "None"
				break
			}
		}
	}

	return entry
}

// entryToRule converts an editable entry back to a NotificationRule.
func entryToRule(entry notificationRuleEntry, servers []serverEntry) NotificationRule {
	rule := NotificationRule{
		Pattern: entry.pattern,
		Source:  entry.source,
	}

	// Convert action index to RuleAction
	switch entry.actionIdx {
	case 0:
		rule.Action = RuleActionNormal
	case 1:
		rule.Action = RuleActionSilent
	case 2:
		rule.Action = RuleActionDismiss
	}

	// Server
	if entry.serverIdx > 0 && int(entry.serverIdx-1) < len(servers) {
		server := servers[entry.serverIdx-1]
		if server.label != "" {
			rule.Server = server.label
		} else {
			rule.Server = server.url
		}
	}

	// Status
	if entry.statusIdx > 0 && int(entry.statusIdx) < len(statusOptions) {
		rule.Status = statusOptions[entry.statusIdx]
	}

	// Sound
	if entry.soundIdx == 0 {
		rule.Sound = ""
	} else if int(entry.soundIdx) < len(soundOptions) {
		rule.Sound = soundOptions[entry.soundIdx]
	}

	return rule
}
