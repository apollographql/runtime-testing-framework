package latency

import "time"

type baseLatencySummand struct {
	latency time.Duration
}

func (b baseLatencySummand) getLatency(elapsed time.Duration) time.Duration {
	return b.latency
}

type LatencyCfg struct {
	Base              time.Duration
	SineAmplitude     time.Duration
	SinePeriod        time.Duration
	SawAmplitude      time.Duration
	SawPeriod         time.Duration
	SquareAmplitude   time.Duration
	SquarePeriod      time.Duration
	TriangleAmplitude time.Duration
	TrianglePeriod    time.Duration
}

type LatencyGenerator interface {
	generateLatency(time.Time) time.Duration
}

type latencySummand interface {
	getLatency(elapsed time.Duration) time.Duration
}

type SimpleLatencyGenerator struct {
	start    time.Time
	summands []latencySummand
}

func NewSimpleLatencyGenerator(start time.Time, cfg *LatencyCfg) SimpleLatencyGenerator {
	summands := []latencySummand{baseLatencySummand{cfg.Base}}
	if cfg.SineAmplitude > 0 && cfg.SinePeriod > 0 {
		summands = append(summands, sineLatencySummand{
			cfg.SineAmplitude,
			cfg.SinePeriod,
		})
	}
	if cfg.SawAmplitude > 0 && cfg.SawPeriod > 0 {
		summands = append(summands, sawtoothLatencySummand{
			cfg.SawAmplitude,
			cfg.SawPeriod,
		})
	}
	if cfg.SquareAmplitude > 0 && cfg.SquarePeriod > 0 {
		summands = append(summands, squareLatencySummand{
			cfg.SquareAmplitude,
			cfg.SquarePeriod,
		})
	}
	if cfg.TriangleAmplitude > 0 && cfg.TrianglePeriod > 0 {
		summands = append(summands, triangleLatencySummand{
			cfg.TriangleAmplitude,
			cfg.TrianglePeriod,
		})
	}
	return SimpleLatencyGenerator{
		start:    start,
		summands: summands,
	}
}

func (g SimpleLatencyGenerator) GenerateLatency(when time.Time) time.Duration {
	var latency time.Duration = 0
	elapsed := when.Sub(g.start)
	for _, s := range g.summands {
		latency += s.getLatency(elapsed)
	}
	return latency
}
