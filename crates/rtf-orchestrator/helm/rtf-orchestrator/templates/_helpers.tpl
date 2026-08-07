{{/*
Expand the name of the chart.
*/}}
{{- define "rtf-orchestrator.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "rtf-orchestrator.fullname" -}}
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
{{- define "rtf-orchestrator.chart" -}}
{{- printf "%s-%s" (include "rtf-orchestrator.name" .) (.Chart.Version | replace "+" "_") | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "rtf-orchestrator.labels" -}}
helm.sh/chart: {{ include "rtf-orchestrator.chart" . }}
{{ include "rtf-orchestrator.selectorLabels" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/version: {{ .Chart.Version | replace "+" "_" | quote }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "rtf-orchestrator.selectorLabels" -}}
app.kubernetes.io/name: {{ include "rtf-orchestrator.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Return the serviceAccount name
*/}}
{{- define "rtf-orchestrator.serviceAccountName" -}}
{{- if .Values.serviceAccount.name }}
{{- .Values.serviceAccount.name }}
{{- else }}
{{- include "rtf-orchestrator.fullname" . }}
{{- end }}
{{- end }}
