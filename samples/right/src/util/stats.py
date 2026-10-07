from statistics import mean


def summarize(values: list[float]) -> dict:
    """Return basic statistics."""
    return {"min": min(values), "max": max(values), "mean": mean(values)}
