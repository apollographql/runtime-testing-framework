{{/*
Expand the name of the chart.
*/}}
{{- define "orchestrator-ui.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "orchestrator-ui.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "orchestrator-ui.labels" -}}
helm.sh/chart: {{ include "orchestrator-ui.name" . }}-{{ .Chart.Version | replace "+" "_" }}
{{ include "orchestrator-ui.selectorLabels" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/version: {{ .Chart.Version | replace "+" "_" | quote }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "orchestrator-ui.selectorLabels" -}}
app.kubernetes.io/name: {{ include "orchestrator-ui.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}
