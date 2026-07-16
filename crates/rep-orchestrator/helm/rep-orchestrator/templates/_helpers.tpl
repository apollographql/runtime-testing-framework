{{/*
Expand the name of the chart.
*/}}
{{- define "rep-orchestrator.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "rep-orchestrator.fullname" -}}
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
{{- define "rep-orchestrator.chart" -}}
{{- printf "%s-%s" (include "rep-orchestrator.name" .) (.Chart.Version | replace "+" "_") | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "rep-orchestrator.labels" -}}
helm.sh/chart: {{ include "rep-orchestrator.chart" . }}
{{ include "rep-orchestrator.selectorLabels" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/version: {{ .Chart.Version | replace "+" "_" | quote }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "rep-orchestrator.selectorLabels" -}}
app.kubernetes.io/name: {{ include "rep-orchestrator.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Return the serviceAccount name
*/}}
{{- define "rep-orchestrator.serviceAccountName" -}}
{{- if .Values.serviceAccount.name }}
{{- .Values.serviceAccount.name }}
{{- else }}
{{- include "rep-orchestrator.fullname" . }}
{{- end }}
{{- end }}
