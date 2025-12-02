// ABOUTME: Defines the WebSocket message protocol for client-server communication.
// ABOUTME: Supports notification delivery, action requests, and resolution broadcasts.

package main

import "encoding/json"

// MessageType identifies the type of WebSocket message.
type MessageType string

const (
	MessageTypeNotification MessageType = "notification"
	MessageTypeAction       MessageType = "action"
	MessageTypeResolved     MessageType = "resolved"
)

// Message is the envelope for all WebSocket communication.
type Message struct {
	Type MessageType     `json:"type"`
	Data json.RawMessage `json:"data"`
}

// ActionMessage is sent by clients when they click an action button.
type ActionMessage struct {
	NotificationID string `json:"id"`
	ActionIndex    int    `json:"actionIndex"`
}

// ResolvedMessage is broadcast by the server when an exclusive notification is resolved.
type ResolvedMessage struct {
	NotificationID string `json:"id"`
	ResolvedBy     string `json:"resolvedBy"`
	ActionLabel    string `json:"actionLabel"`
	Success        bool   `json:"success"`
	Error          string `json:"error,omitempty"`
}

// EncodeMessage creates a Message envelope with the given type and data.
func EncodeMessage(msgType MessageType, data interface{}) ([]byte, error) {
	dataBytes, err := json.Marshal(data)
	if err != nil {
		return nil, err
	}
	msg := Message{
		Type: msgType,
		Data: dataBytes,
	}
	return json.Marshal(msg)
}

// DecodeMessage parses a raw message into type and data components.
func DecodeMessage(raw []byte) (*Message, error) {
	var msg Message
	if err := json.Unmarshal(raw, &msg); err != nil {
		return nil, err
	}
	return &msg, nil
}
