{{/*
Expand the name of the chart.
*/}}
{{- define "rtf-orchestrator-ui.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "rtf-orchestrator-ui.fullname" -}}
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
Chart name and version, truncated to fit the 63-byte Kubernetes label limit.
*/}}
{{- define "rtf-orchestrator-ui.chart" -}}
{{- printf "%s-%s" (include "rtf-orchestrator-ui.name" .) (.Chart.Version | replace "+" "_") | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "rtf-orchestrator-ui.labels" -}}
helm.sh/chart: {{ include "rtf-orchestrator-ui.chart" . }}
{{ include "rtf-orchestrator-ui.selectorLabels" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/version: {{ .Chart.Version | replace "+" "_" | quote }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "rtf-orchestrator-ui.selectorLabels" -}}
app.kubernetes.io/name: {{ include "rtf-orchestrator-ui.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}
