﻿using ApiService.OneFuzzLib.Orm;
using Microsoft.Extensions.Logging;
using Polly;
namespace Microsoft.OneFuzz.Service;

public interface IJobResultOperations : IOrm<JobResult> {

    Async.Task<JobResult?> GetJobResult(Guid jobId);
    Async.Task<OneFuzzResultVoid> CreateOrUpdate(Guid jobId, JobResultType resultType, Dictionary<string, double> resultValue);

}
public class JobResultOperations : Orm<JobResult>, IJobResultOperations {

    public JobResultOperations(ILogger<JobResultOperations> log, IOnefuzzContext context)
        : base(log, context) {
    }

    public async Async.Task<JobResult?> GetJobResult(Guid jobId) {
        return await SearchByPartitionKeys(new[] { jobId.ToString() }).SingleOrDefaultAsync();
    }

    private JobResult UpdateResult(JobResult result, JobResultType type, Dictionary<string, double> resultValue) {

        var newResult = result;
        double newValue;
        switch (type) {
            case JobResultType.NewCrashingInput:
                newValue = result.NewCrashingInput + resultValue["count"];
                newResult = result with { NewCrashingInput = newValue };
                break;
            case JobResultType.NewReport:
                newValue = result.NewReport + resultValue["count"];
                newResult = result with { NewReport = newValue };
                break;
            case JobResultType.NewUniqueReport:
                newValue = result.NewUniqueReport + resultValue["count"];
                newResult = result with { NewUniqueReport = newValue };
                break;
            case JobResultType.NewRegressionReport:
                newValue = result.NewRegressionReport + resultValue["count"];
                newResult = result with { NewRegressionReport = newValue };
                break;
            case JobResultType.NewCrashDump:
                newValue = result.NewCrashDump + resultValue["count"];
                newResult = result with { NewCrashDump = newValue };
                break;
            case JobResultType.CoverageData:
                double newCovered = resultValue["covered"];
                double newTotalCovered = resultValue["features"];
                double newCoverageRate = resultValue["rate"];
                newResult = result with { InstructionsCovered = newCovered, TotalInstructions = newTotalCovered, CoverageRate = newCoverageRate };
                break;
            case JobResultType.RuntimeStats:
                double newTotalIterations = resultValue["total_count"];
                newResult = result with { IterationCount = newTotalIterations };
                break;
            default:
                _logTracer.LogWarning($"Invalid Field {type}.");
                break;
        }
        _logTracer.LogInformation($"Attempting to log new result: {newResult}");
        return newResult;
    }

    private async Async.Task<bool> TryUpdate(Job job, JobResultType resultType, Dictionary<string, double> resultValue) {
        var jobId = job.JobId;

        var jobResult = await GetJobResult(jobId);

        if (jobResult == null) {
            _logTracer.LogInformation("Creating new JobResult for Job {JobId}", jobId);

            var entry = new JobResult(JobId: jobId, Project: job.Config.Project, Name: job.Config.Name);

            jobResult = UpdateResult(entry, resultType, resultValue);

            var r = await Insert(jobResult);
            if (!r.IsOk) {
                throw new InvalidOperationException($"failed to insert job result {jobResult.JobId}");
            }
            _logTracer.LogInformation("created job result {JobId}", jobResult.JobId);
        } else {
            _logTracer.LogInformation("Updating existing JobResult entry for Job {JobId}", jobId);

            jobResult = UpdateResult(jobResult, resultType, resultValue);

            var r = await Update(jobResult);
            if (!r.IsOk) {
                throw new InvalidOperationException($"failed to insert job result {jobResult.JobId}");
            }
            _logTracer.LogInformation("updated job result {JobId}", jobResult.JobId);
        }

        return true;
    }

    public async Async.Task<OneFuzzResultVoid> CreateOrUpdate(Guid jobId, JobResultType resultType, Dictionary<string, double> resultValue) {

        var job = await _context.JobOperations.Get(jobId);
        if (job == null) {
            return OneFuzzResultVoid.Error(ErrorCode.INVALID_REQUEST, "invalid job");
        }

        var success = false;
        try {
            _logTracer.LogInformation("attempt to update job result {JobId}", job.JobId);
            var policy = Policy.Handle<InvalidOperationException>().WaitAndRetryAsync(50, _ => new TimeSpan(0, 0, 5));
            await policy.ExecuteAsync(async () => {
                success = await TryUpdate(job, resultType, resultValue);
                _logTracer.LogInformation("attempt {success}", success);
            });
            return OneFuzzResultVoid.Ok;
        } catch (Exception e) {
            return OneFuzzResultVoid.Error(ErrorCode.UNABLE_TO_UPDATE, new string[] {
                    $"Unexpected failure when attempting to update job result for {job.JobId}",
                    $"Exception: {e}"
                });
        }
    }
}

